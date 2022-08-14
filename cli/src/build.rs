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
use std::fmt::Write;
use std::{
    collections::HashMap,
    fs,
    ops::Range,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::flash;
use crate::image::{Image, ImageBuilder};
use crate::image::{SRecImage, SRecImageBuilder};

pub static ARM_TASK_RLINK_BYTES: &[u8] = include_bytes!("task-rlink.x");
pub static ARM_TASK_LINK_BYTES: &[u8] = include_bytes!("task-link.x");
pub static RV_TASK_RLINK_BYTES: &[u8] = include_bytes!("rv-task-rlink.x");
pub static RV_TASK_LINK_BYTES: &[u8] = include_bytes!("rv-task-link.x");
pub static KERN_LINK_BYTES: &[u8] = include_bytes!("kern-link.x");
pub static KERN_RV_LINK_BYTES: &[u8] = include_bytes!("kern-rv-link.x");

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(flatten)]
    pub flash_probe: flash::FlashConfig,
    pub tasks: Vec<Task>,
    pub regions: HashMap<String, MemorySection>,
    stack_size: Option<usize>,
    stack_space_size: Option<usize>,
    pub kernel: Kernel,
    platform: Platform,
}

#[derive(Debug, Deserialize)]
pub struct Task {
    pub name: String,
    #[serde(flatten)]
    pub source: TaskSource,
    #[allow(dead_code)]
    pub secure: bool,
    // TODO(sphw): this will be used to position secure tasks above the kernel
    #[serde(default)]
    pub stack_size: usize,
    #[serde(default)]
    pub stack_space_size: usize,
}

#[derive(Debug, Deserialize)]
pub struct Kernel {
    pub crate_path: PathBuf,
    #[serde(default)]
    stack_size: usize,
    pub(crate) sizes: HashMap<String, usize>,
    linker_script: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Copy, Clone)]
pub enum Platform {
    RV32,
    AwD1,
    ArmV8m,
}

impl Platform {
    pub(crate) fn kern_link(&self) -> &'static [u8] {
        match self {
            Platform::RV32 => KERN_RV_LINK_BYTES,
            Platform::AwD1 => {
                todo!()
            }
            Platform::ArmV8m => KERN_LINK_BYTES,
        }
    }

    pub(crate) fn task_rlink(&self) -> &'static [u8] {
        match self {
            Platform::AwD1 | Platform::RV32 => RV_TASK_RLINK_BYTES,
            Platform::ArmV8m => ARM_TASK_RLINK_BYTES,
        }
    }

    pub(crate) fn task_link(&self) -> &'static [u8] {
        match self {
            Platform::AwD1 | Platform::RV32 => RV_TASK_LINK_BYTES,
            Platform::ArmV8m => ARM_TASK_LINK_BYTES,
        }
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryRole {
    None,
    Stack,
}

impl Default for MemoryRole {
    fn default() -> Self {
        MemoryRole::None
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum TaskSource {
    Crate { crate_path: PathBuf },
}

impl Config {
    pub fn build(&mut self, app_path: &Path) -> Result<PathBuf> {
        if self.kernel.crate_path.is_relative() {
            self.kernel.crate_path =
                fs::canonicalize(app_path.join(self.kernel.crate_path.clone()))?;
        }
        if let Some(linker_path) = &mut self.kernel.linker_script {
            if linker_path.is_relative() {
                *linker_path = fs::canonicalize(app_path.join(linker_path.clone()))?;
            }
        }

        for task in &mut self.tasks {
            if task.stack_size == 0 {
                task.stack_size = self
                    .stack_size
                    .ok_or_else(|| anyhow!("missing default stack size"))?;
            }
            if task.stack_space_size == 0 {
                task.stack_space_size = self
                    .stack_space_size
                    .ok_or_else(|| anyhow!("missing default stack space size"))?;
            }
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

        let target_path = self.kernel.crate_path.join("target");
        let mut builder: Box<dyn ImageBuilder<Image = SRecImage>> = match self.platform {
            // Platform::AwD1 => Box::new(D1ImageBuilder::new(
            //     self.regions.clone(),
            //     self.platform,
            //     &self.kernel,
            // )?),
            _ => Box::new(SRecImageBuilder::new(
                self.regions.clone(),
                self.platform,
                &self.kernel,
            )),
        };
        for task in &self.tasks {
            builder.task(task)?;
        }
        builder.kernel(&self.kernel)?;
        let img = builder.build()?;
        img.write(&target_path)?;
        Ok(target_path)
    }
}

impl Kernel {
    pub(crate) fn build(
        &self,
        platform: Platform,
        regions: HashMap<String, MemorySection>,
        tasks: Vec<codegen::Task>,
    ) -> Result<PathBuf> {
        crate::print_header("Building kernel");
        let target_dir = self.crate_path.join("target");
        fs::create_dir_all(&target_dir)?;
        let kern_loc = TaskLoc { regions };
        fs::write(
            target_dir.join("memory.x"),
            kern_loc.memory_linker_script(self.stack_size)?.as_bytes(),
        )?;
        if let Some(kern_link_path) = &self.linker_script {
            println!("copying {:?} {:?}", kern_link_path, self.linker_script);
            fs::copy(kern_link_path, target_dir.join("link.x"))?;
        } else {
            fs::write(target_dir.join("link.x"), platform.kern_link())?;
        }
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
    if std::env::var("DEFMT_LOG").is_err() {
        cmd.env("DEFMT_LOG", "debug");
    }
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
    pub fn target_dir(&self) -> PathBuf {
        let TaskSource::Crate { crate_path } = &self.source;
        crate_path.join("target")
    }

    pub fn build(&self, plat: Platform) -> Result<PathBuf> {
        crate::print_header(format!("Building {}", self.name));
        let TaskSource::Crate { crate_path } = &self.source;

        let target_dir = crate_path.join("target");
        fs::create_dir_all(&target_dir)?;
        fs::write(target_dir.join("link.x"), plat.task_rlink())?;
        build_crate(crate_path, true, None)
    }

    pub fn link(
        &self,
        reloc_elf: &Path,
        dest: &Path,
        task_loc: &TaskLoc,
        link_script: &[u8],
    ) -> Result<()> {
        let target_dir = self.target_dir();
        println!("{:?}", task_loc);
        fs::write(
            target_dir.join("memory.x"),
            task_loc
                .memory_linker_script(self.stack_space_size)?
                .as_bytes(),
        )?;
        fs::write(target_dir.join("link.x"), link_script)?;
        let status = Command::new("riscv64-unknown-elf-ld")
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

pub(crate) fn get_elf_size(
    elf: &Path,
    regions: &HashMap<String, MemorySection>,
    stack_space_size: usize,
) -> Result<HashMap<String, Range<usize>>> {
    let elf = fs::read(elf)?;
    let elf = if let Object::Elf(e) = Object::parse(&elf)? {
        e
    } else {
        return Err(anyhow!("object must be an elf"));
    };
    let mut sizes = HashMap::new();
    let mut add_section = |start, size| {
        for (name, region) in regions.iter() {
            if region.contains(start) {
                let end = start + size;
                let range = sizes.entry(name.clone()).or_insert(start..end);
                range.start = range.start.min(start);
                range.end = range.end.max(end);
                return true;
            }
        }
        false
    };
    for header in &elf.program_headers {
        add_section(header.p_vaddr as usize, header.p_memsz as usize);
        if header.p_vaddr != header.p_paddr
            && !add_section(header.p_paddr as usize, header.p_filesz as usize)
        {
            return Err(anyhow!("failed to remap relocated section"));
        }
    }
    let (stack_name, _) = regions
        .iter()
        .find(|(_, r)| r.role == MemoryRole::Stack)
        .ok_or_else(|| {
            anyhow!("no stack region found. Make sure to specify a signle region for the stack")
        })?;
    let stack_range = sizes.entry(stack_name.clone()).or_insert(0..0);
    stack_range.end = stack_range.end + stack_space_size;
    Ok(sizes)
}

#[derive(Debug)]
pub struct TaskLoc {
    pub(crate) regions: HashMap<String, MemorySection>,
}

impl TaskLoc {
    fn memory_linker_script(&self, stack_size: usize) -> Result<String> {
        let mut file = "MEMORY {\n".to_string();
        for (name, section) in self.regions.iter() {
            let mut section = section.clone();
            if stack_size != 0 && section.role == MemoryRole::Stack {
                writeln!(
                    &mut file,
                    "STACK : ORIGIN = {:#010x}, LENGTH = {:#010x}",
                    section.address, stack_size
                )?;
                section.address = section.address + stack_size;
                section.size = section.size - stack_size;
            }
            writeln!(
                &mut file,
                "{} : ORIGIN = {:#010x}, LENGTH = {:#010x}",
                name.to_uppercase(),
                section.address,
                section.size
            )?;
        }
        file += "}";
        Ok(file)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct MemorySection {
    pub address: usize,
    pub size: usize,
    #[serde(default)]
    pub role: MemoryRole,
}

impl MemorySection {
    fn contains(&self, addr: usize) -> bool {
        addr >= self.address && addr <= (self.address + self.size)
    }
}

#[derive(Default)]
pub struct SRecWriter {
    pub buf: Vec<srec::Record>,
}

impl SRecWriter {
    pub(crate) fn write(&mut self, elf: &Path) -> Result<usize> {
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
                self.buf.push(srec::Record::S3(srec::Data {
                    address: srec::Address32(addr),
                    data: chunk.to_vec(),
                }));
                addr += chunk.len() as u32;
            }
        }
        Ok(elf.header.e_entry as usize)
    }

    pub(crate) fn write_slice(&mut self, mut addr: usize, buf: &[u8]) {
        for chunk in buf.chunks(250) {
            self.buf.push(srec::Record::S3(srec::Data {
                address: srec::Address32(addr as u32),
                data: chunk.to_vec(),
            }));
            addr += chunk.len()
        }
    }

    pub(crate) fn finalize(&mut self) -> String {
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
pub const fn align_up(addr: usize, align: usize) -> usize {
    assert!(align.is_power_of_two(), "`align` must be a power of two");
    let align_mask = align - 1;
    if addr & align_mask == 0 {
        addr // already aligned
    } else {
        (addr | align_mask) + 1
    }
}
