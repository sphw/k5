use color_eyre::eyre::anyhow;
use color_eyre::Result;
use std::{
    collections::HashMap,
    fs::{self},
    path::Path,
    process::Command,
};

mod egon;
pub use egon::*;

use crate::build::{
    align_up, get_elf_size, Kernel, MemoryRole, MemorySection, Platform, SRecWriter, Task, TaskLoc,
    TASK_LINK_BYTES,
};

pub(crate) trait ImageBuilder {
    type Image;
    fn task(&mut self, task: &Task) -> Result<()>;
    fn kernel(&mut self, kern: &Kernel) -> Result<()>;
    fn build(&mut self) -> Result<Self::Image>;
}

pub trait Image {
    fn write(&self, target_path: &Path) -> Result<()>;
}

pub struct SRecImageBuilder {
    current_locs: HashMap<String, MemorySection>,
    regions: HashMap<String, MemorySection>,
    platform: Platform,
    codegen_tasks: Vec<codegen::Task>,
    output: SRecWriter,
}

impl SRecImageBuilder {
    pub(crate) fn new(
        regions: HashMap<String, MemorySection>,
        platform: Platform,
        kern: &Kernel,
    ) -> Self {
        let mut current_locs = regions.clone();
        for (name, size) in kern.sizes.iter() {
            let loc = &mut current_locs.get_mut(name).unwrap();
            loc.address += size;
        }
        Self {
            current_locs,
            regions,
            platform,
            codegen_tasks: vec![],
            output: SRecWriter::default(),
        }
    }
}

impl ImageBuilder for SRecImageBuilder {
    type Image = SRecImage;

    fn kernel(&mut self, kern: &Kernel) -> Result<()> {
        let kernel_path = kern.build(
            self.platform,
            self.regions.clone(),
            self.codegen_tasks.clone(),
        )?;
        self.output.write(&kernel_path)?;
        fs::copy(
            &kernel_path,
            kern.crate_path.join("target").join("kernel.elf"),
        )?;
        Ok(())
    }

    fn task(&mut self, task: &Task) -> Result<()> {
        let reloc = task.build()?;
        let elf = &task.target_dir().join("size.elf");
        task.link(
            &reloc,
            elf,
            &TaskLoc {
                regions: self.regions.clone(),
            },
            TASK_LINK_BYTES,
        )?;
        let sizes = get_elf_size(elf, &self.regions, task.stack_space_size)?;
        let regions: HashMap<_, _> = sizes
            .clone()
            .into_iter()
            .map(|(name, range)| {
                (
                    name.clone(),
                    MemorySection {
                        size: align_up(range.len(), 32),
                        ..self.current_locs[&name]
                    },
                )
            })
            .collect();
        for (name, size) in sizes.iter() {
            let loc = &mut self.current_locs.get_mut(name).unwrap();
            loc.address += align_up(size.len(), 32);
        }
        println!("{:?}", self.current_locs);
        let elf = task.target_dir().join("final.elf");
        task.link(
            &reloc,
            &elf,
            &TaskLoc {
                regions: regions.clone(),
            },
            TASK_LINK_BYTES,
        )?;

        let entrypoint = self.output.write(&elf)?;
        let stack_region = regions
            .values()
            .find(|r| r.role == MemoryRole::Stack)
            .ok_or_else(|| {
                anyhow!("no stack region found. Make sure to specify a signle region for the stack")
            })?;
        self.codegen_tasks.push(codegen::Task {
            name: task.name.clone(),
            entrypoint,
            stack_space: stack_region.address..stack_region.address + task.stack_space_size,
            init_stack_size: task.stack_size,
            regions: regions
                .values()
                .map(|r| r.address..r.address + r.size)
                .collect(),
        });
        Ok(())
    }

    fn build(&mut self) -> Result<Self::Image> {
        Ok(Self::Image {
            srec: self.output.finalize(),
        })
    }
}

pub struct SRecImage {
    srec: String,
}
impl Image for SRecImage {
    fn write(&self, target_path: &Path) -> Result<()> {
        let out_path = target_path.join("final.srec");
        fs::write(&out_path, &self.srec)?;
        let ihex_path = target_path.join("final.ihex");
        let output = Command::new("arm-none-eabi-objcopy")
            .arg("-Isrec")
            .arg(&out_path)
            .arg(ihex_path)
            .arg("-Oihex")
            .output()?;

        if !output.status.success() {
            return Err(anyhow!(
                "objcopy failed: {:?}",
                std::str::from_utf8(&output.stderr)
            ));
        }
        let bin_path = target_path.join("final.bin");
        let output = Command::new("arm-none-eabi-objcopy")
            .arg("-Isrec")
            .arg(&out_path)
            .arg(bin_path)
            .arg("-Obinary")
            .output()?;
        if !output.status.success() {
            return Err(anyhow!(
                "objcopy failed: {:?}",
                std::str::from_utf8(&output.stderr)
            ));
        }
        Ok(())
    }
}
