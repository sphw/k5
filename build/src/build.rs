///! Utilities to build k5 binaries
///!
///! This file contains the logic that packs tasks and the kernel together into a
///! single image
///!
///! The scheme, and much of the logic is borrowed from Hubris OS's xtask dist.rs file.
///! It is not identical, but uses the same flow for auto-sizing tasks. Notable differences are
///! that hubris supports multiple memory sections. Hubris also has a more complex Kernel
///! configuration, due to how they enforce peripheral and memory isolation.
///!
///! For now this utility soley supports ARM-V8M, since it has relaxed alignment reqs.
///! RISC-V and ARM-V7M are next on the docket.
use cargo_metadata::Message;
use color_eyre::{eyre::anyhow, Result};
use goblin::{elf64::program_header::PT_LOAD, Object};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    ops::Range,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

static TASK_RLINK_BYTES: &[u8] = include_bytes!("task-rlink.x");
static TASK_TLINK_BYTES: &[u8] = include_bytes!("task-tlink.x");
static TASK_LINK_BYTES: &[u8] = include_bytes!("task-link.x");
static KERN_LINK_BYTES: &[u8] = include_bytes!("kern-link.x");

#[derive(Debug, Deserialize)]
pub struct Config {
    tasks: Vec<Task>,
    flash: MemorySection,
    ram: MemorySection,
    stack_size: Option<u32>,
    stack_space_size: Option<u32>,
    kernel: Kernel,
}

#[derive(Debug, Deserialize)]
struct Task {
    name: String,
    #[serde(flatten)]
    source: TaskSource,
    #[allow(dead_code)]
    secure: bool,
    // TODO(sphw): this will be used to position secure tasks above the kernel
    #[serde(default)]
    stack_size: u32,
    #[serde(default)]
    stack_space_size: u32,
}

#[derive(Debug, Deserialize)]
struct Kernel {
    crate_path: PathBuf,
    #[serde(default)]
    stack_size: u32,
    flash_size: u32,
    ram_size: u32,
}

struct TaskTableEntry<'a> {
    task: &'a Task,
    loc: TaskLoc,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TaskSource {
    Crate { crate_path: PathBuf },
}

impl Config {
    pub fn build(&mut self, app_path: &PathBuf) -> Result<()> {
        if self.kernel.crate_path.is_relative() {
            self.kernel.crate_path =
                fs::canonicalize(app_path.join(self.kernel.crate_path.clone()))?;
        }

        for task in &mut self.tasks {
            if task.stack_size == 0 {
                task.stack_size = self
                    .stack_size
                    .ok_or_else(|| anyhow!("missing default stack size"))?;
            }
            if task.stack_space_size == 0 {
                println!("setting stack space {:?}", self.stack_space_size);
                task.stack_space_size = self
                    .stack_space_size
                    .ok_or_else(|| anyhow!("missing default stack space size"))?;
            }
            println!("{:?}", task);
            match task.source {
                TaskSource::Crate { ref mut crate_path } => {
                    if crate_path.is_relative() {
                        *crate_path = fs::canonicalize(app_path.join(crate_path.clone()))?;
                    }
                }
            }
        }
        let mut task_by_name = HashMap::new();
        for (i, task) in self.tasks.iter().enumerate() {
            if task_by_name.insert(task.name.clone(), i).is_some() {
                return Err(anyhow!("{:?} appears more than once", task.name));
            }
        }
        if !task_by_name.contains_key("idle") {
            return Err(anyhow!("missing idle task"));
        }

        let relocs = self
            .tasks
            .iter()
            .map(Task::build)
            .collect::<Result<Vec<_>, _>>()?;
        let full_size_loc = TaskLoc {
            flash: self.flash.clone(),
            ram: self.ram.clone(),
        };
        let mut current_flash_loc = self.flash.address + self.kernel.flash_size;
        let mut current_ram_loc = self.ram.address + self.kernel.ram_size;
        let task_table = relocs
            .iter()
            .zip(self.tasks.iter())
            .map(|(reloc, task)| {
                let elf = &task.target_dir().join("size.elf");
                task.link(reloc, &elf, &full_size_loc, TASK_TLINK_BYTES)?;
                let size = get_elf_size(elf, &self.flash, &self.ram, task.stack_space_size)?;
                println!("task {:?} size: {:?}", task.name, size);
                let entry = TaskTableEntry {
                    task,
                    loc: TaskLoc {
                        flash: MemorySection {
                            address: current_flash_loc,
                            size: size.flash,
                        },
                        ram: MemorySection {
                            address: current_ram_loc,
                            size: size.ram,
                        },
                    },
                };
                current_flash_loc += size.flash;
                current_ram_loc += size.ram;
                Ok(entry)
            })
            .collect::<Result<Vec<_>>>()?;
        let final_tasks = relocs
            .iter()
            .zip(task_table.iter())
            .map(|(reloc, entry)| {
                println!("linking final: {:?}", entry.task.name);
                let elf = entry.task.target_dir().join("final.elf");
                entry.task.link(reloc, &elf, &entry.loc, TASK_LINK_BYTES)?;
                Ok(elf)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut output = SRecWriter::default();
        let codegen_tasks = final_tasks
            .iter()
            .zip(task_table.iter())
            .map(|(elf, entry)| {
                let TaskTableEntry { task, loc } = entry;
                println!("writing task: {:?} {:?}", task.name, elf);
                let entrypoint = output.write(&elf)?;
                Ok(codegen::Task {
                    name: task.name.clone(),
                    entrypoint,
                    stack_space: loc.ram.address..loc.ram.address + task.stack_space_size,
                    init_stack_size: task.stack_size,
                    ram_region: loc.ram.address..loc.ram.address + loc.ram.size,
                    flash_region: loc.flash.address..loc.flash.address + loc.flash.size,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let kernel = self
            .kernel
            .build(self.flash.address, self.ram.address, codegen_tasks)?;
        output.write(&kernel)?;

        let out = output.finalize();
        let out_path = self.kernel.crate_path.join("target").join("final.srec");
        fs::write(out_path, &out)?;

        Ok(())
    }
}

impl Kernel {
    fn build(&self, flash: u32, ram: u32, tasks: Vec<codegen::Task>) -> Result<PathBuf> {
        let target_dir = self.crate_path.join("target");
        fs::create_dir_all(&target_dir)?;
        let kern_loc = TaskLoc {
            flash: MemorySection {
                address: flash,
                size: self.flash_size,
            },
            ram: MemorySection {
                address: ram,
                size: self.ram_size,
            },
        };
        fs::write(
            target_dir.join("memory.x"),
            kern_loc.memory_linker_script(self.stack_size).as_bytes(),
        )?;
        fs::write(target_dir.join("link.x"), KERN_LINK_BYTES)?;
        let task_list = codegen::TaskList { tasks };
        let task_list_path = target_dir.join("task_list.json");
        fs::write(task_list_path.clone(), serde_json::to_vec(&task_list)?)?;
        build_crate(&self.crate_path, false, Some(&task_list_path))
    }
}

fn build_crate(crate_path: &Path, relocate: bool, task_list: Option<&Path>) -> Result<PathBuf> {
    let target_dir = crate_path.join("target");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&crate_path)
        .arg("rustc")
        .args(&["--message-format", "json-diagnostic-rendered-ansi"])
        .arg("--")
        .arg("-C")
        .arg("link-arg=-Tlink.x")
        .arg("-L")
        .arg(format!("{}", target_dir.display()));
    if relocate {
        cmd.arg("-C").arg("link-arg=-r");
    };
    if let Some(task_list) = task_list {
        cmd.env("K5_TASK_LIST", task_list);
    }
    let cmd = cmd.stdout(Stdio::piped()).spawn()?;
    let output = cmd.wait_with_output()?;

    let msgs = Message::parse_stream(&output.stdout[..]);
    let mut target_artifact = None;
    for msg in msgs {
        match msg? {
            Message::CompilerArtifact(artifact) => {
                if let Some(e) = artifact.executable {
                    if target_artifact.is_some() {
                        return Err(anyhow!("too many artifacts"));
                    }
                    target_artifact = Some(PathBuf::from(e.as_path()));
                }
            }
            Message::CompilerMessage(msg) => {
                if let Some(rendered) = msg.message.rendered {
                    print!("{}", rendered);
                }
            }
            _ => {}
        }
    }

    if !output.status.success() {
        return Err(anyhow!(
            "cargo exited with status: {:?}",
            output.status.code()
        ));
    }
    if let Some(a) = target_artifact {
        Ok(a)
    } else {
        Err(anyhow!("artifact not found"))
    }
}

impl Task {
    fn target_dir(&self) -> PathBuf {
        let TaskSource::Crate { crate_path } = &self.source;
        crate_path.join("target")
    }

    fn build(&self) -> Result<PathBuf> {
        let TaskSource::Crate { crate_path } = &self.source;

        let target_dir = crate_path.join("target");
        fs::create_dir_all(&target_dir)?;
        fs::write(target_dir.join("link.x"), TASK_RLINK_BYTES)?;
        build_crate(&crate_path, true, None)
    }

    fn link(
        &self,
        reloc_elf: &Path,
        dest: &Path,
        task_loc: &TaskLoc,
        link_script: &[u8],
    ) -> Result<()> {
        let target_dir = self.target_dir();
        fs::write(
            target_dir.join("memory.x"),
            task_loc
                .memory_linker_script(self.stack_space_size)
                .as_bytes(),
        )?;
        fs::write(target_dir.join("link.x"), link_script)?;
        let status = Command::new("arm-none-eabi-ld")
            .current_dir(target_dir)
            .arg(reloc_elf)
            .arg("-o")
            .arg(dest)
            .arg("-Tlink.x")
            .arg("--gc-sections")
            .arg("-z")
            .arg("common-page-size=0x20")
            .arg("-z")
            .arg("max-page-size=0x20")
            .status()?;
        if !status.success() {
            return Err(anyhow!("link failed"));
        }
        Ok(())
    }
}

fn get_elf_size(
    elf: &Path,
    flash: &MemorySection,
    ram: &MemorySection,
    stacksize: u32,
) -> Result<TaskSize> {
    let elf = fs::read(elf)?;
    let elf = if let Object::Elf(e) = Object::parse(&elf)? {
        e
    } else {
        return Err(anyhow!("object must be an elf"));
    };
    let mut flash_range: Option<Range<u32>> = None;
    let mut ram_range: Option<Range<u32>> = None;
    fn expand_range(range: &mut Option<Range<u32>>, start: u32, size: u32) {
        let range = range.get_or_insert(start..size);
        let end = start + size;
        range.start = range.start.min(start);
        range.end = range.end.max(end);
    }
    let mut add_section = |start, size| {
        if flash.contains(start) {
            expand_range(&mut flash_range, start, size);
            true
        } else if ram.contains(start) {
            expand_range(&mut ram_range, start, size);
            true
        } else {
            false
        }
    };
    for header in &elf.program_headers {
        add_section(header.p_vaddr as u32, header.p_memsz as u32);
        if header.p_vaddr != header.p_paddr
            && !add_section(header.p_paddr as u32, header.p_filesz as u32)
        {
            return Err(anyhow!("failed to remap relocated section"));
        }
    }
    let flash_range = flash_range.ok_or_else(|| anyhow!("failed to size flash for task"))?;
    let ram_range = ram_range.unwrap_or_default();
    Ok(TaskSize {
        flash: align_up(flash_range.end - flash_range.start, 32),
        ram: align_up((ram_range.end - ram_range.start) + stacksize, 32),
    })
}

#[derive(Debug)]
struct TaskSize {
    flash: u32,
    ram: u32,
}

#[derive(Debug)]
struct TaskLoc {
    flash: MemorySection,
    ram: MemorySection,
}

impl TaskLoc {
    fn memory_linker_script(&self, stack_size: u32) -> String {
        println!("gen memory linker {:?} {:?}", self, stack_size);
        let ram_start = self.ram.address + stack_size;
        let ram_size = self.ram.size - stack_size;
        format!(
            "MEMORY
{{
FLASH : ORIGIN = {:#010x} , LENGTH = {:#010x}
STACK : ORIGIN = {:#010x}, LENGTH = {:#010x}
RAM : ORIGIN = {:#010x}, LENGTH = {:#010x}
}}",
            self.flash.address, self.flash.size, self.ram.address, stack_size, ram_start, ram_size
        )
    }
}

#[derive(Debug, Deserialize, Clone)]
struct MemorySection {
    pub address: u32,
    pub size: u32,
}

impl MemorySection {
    fn contains(&self, addr: u32) -> bool {
        addr >= self.address && addr <= (self.address + self.size)
    }
}

#[derive(Default)]
pub struct SRecWriter {
    buf: Vec<srec::Record>,
}

impl SRecWriter {
    fn write(&mut self, elf: &Path) -> Result<u32> {
        let image = fs::read(elf)?;
        let elf = if let Object::Elf(e) = Object::parse(&image)? {
            e
        } else {
            return Err(anyhow!("object must be an elf"));
        };
        for header in &elf.program_headers {
            if header.p_type != PT_LOAD {
                continue;
            }
            let data =
                &image[header.p_offset as usize..(header.p_offset + header.p_filesz) as usize];
            let mut addr = header.p_paddr as u32;
            for chunk in data.chunks(250) {
                println!("adding rec for : {:x?} {:?}", addr, chunk.len());
                self.buf.push(srec::Record::S3(srec::Data {
                    address: srec::Address32(addr),
                    data: chunk.to_vec(),
                }));
                addr += chunk.len() as u32;
            }
        }
        Ok(elf.header.e_entry as u32)
    }

    fn kern_entry(&mut self, loc: u32) {
        self.buf.push(srec::Record::S7(srec::Address32(loc)));
    }

    fn finalize(mut self) -> String {
        let sec_count = self.buf.len();
        if sec_count < 0x1_00_00 {
            self.buf
                .push(srec::Record::S5(srec::Count16(sec_count as u16)));
        } else if sec_count < 0x1_00_00_00 {
            self.buf
                .push(srec::Record::S6(srec::Count24(sec_count as u32)));
        } else {
            panic!("srec limit exceeded");
        }
        srec::writer::generate_srec_file(&self.buf)
    }
}

// source: https://docs.rs/x86_64/latest/x86_64/addr/fn.align_up.html
#[inline]
pub const fn align_up(addr: u32, align: u32) -> u32 {
    assert!(align.is_power_of_two(), "`align` must be a power of two");
    let align_mask = align - 1;
    if addr & align_mask == 0 {
        addr // already aligned
    } else {
        (addr | align_mask) + 1
    }
}
