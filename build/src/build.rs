use cargo_metadata::Message;
use color_eyre::{eyre::anyhow, Result};
use goblin::Object;
use serde::Deserialize;
use std::{
    fs,
    ops::Range,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

static TASK_RLINK_BYTES: &[u8] = include_bytes!("task-rlink.x");
static TASK_TLINK_BYTES: &[u8] = include_bytes!("task-Tlink.x");
static TASK_LINK_BYTES: &[u8] = include_bytes!("task-link.x");

#[derive(Debug, Deserialize)]
pub struct Config {
    tasks: Vec<Task>,
    flash: MemorySection,
    ram: MemorySection,
    stack_size: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct Task {
    name: String,
    #[serde(flatten)]
    source: TaskSource,
    secure: bool,
    root: bool,
    #[serde(default)]
    stack_size: u32,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TaskSource {
    Crate { crate_path: PathBuf },
}

impl Config {
    pub fn build(&mut self, app_path: &PathBuf) -> Result<()> {
        for task in &mut self.tasks {
            if task.stack_size == 0 {
                task.stack_size = self
                    .stack_size
                    .ok_or_else(|| anyhow!("missing default stack size"))?;
            }
            match task.source {
                TaskSource::Crate { ref mut crate_path } => {
                    if crate_path.is_relative() {
                        *crate_path = fs::canonicalize(app_path.join(crate_path.clone()))?;
                    }
                }
            }
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
        for (reloc, task) in relocs.iter().zip(self.tasks.iter()) {
            let elf = &task.target_dir().join("size.elf");
            task.link(reloc, &elf, &full_size_loc, TASK_TLINK_BYTES)?;
            let size = get_elf_size(elf, &self.flash, &self.ram, task.stack_size)?;
            println!("task {:?} size: {:?}", task.name, size)
        }
        Ok(())
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
        let cmd = Command::new("cargo")
            .current_dir(&crate_path)
            .arg("rustc")
            .args(&["--message-format", "json-diagnostic-rendered-ansi"])
            .arg("--")
            .arg("-C")
            .arg("link-arg=-Tlink.x")
            .arg("-L")
            .arg(format!("{}", target_dir.display()))
            .arg("-C")
            .arg("link-arg=-r")
            .stdout(Stdio::piped())
            .spawn()?;
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
            task_loc.memory_linker_script(self.stack_size).as_bytes(),
        )?;
        fs::write(target_dir.join("link.x"), link_script)?;
        let status = Command::new("arm-none-eabi-ld")
            .current_dir(target_dir)
            .arg(reloc_elf)
            .arg("-o")
            .arg(dest)
            .arg("-Tlink.x")
            .arg("--gc-sections")
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
        if header.p_vaddr != header.p_paddr {
            if !add_section(header.p_paddr as u32, header.p_filesz as u32) {
                return Err(anyhow!("failed to remap relocated section"));
            }
        }
    }
    let flash_range = flash_range.ok_or_else(|| anyhow!("failed to size flash for task"))?;
    let ram_range = ram_range.unwrap_or_default();
    Ok(TaskSize {
        flash: flash_range.end - flash_range.start,
        ram: (ram_range.end - ram_range.start) + stacksize,
    })
}

#[derive(Debug)]
struct TaskSize {
    flash: u32,
    ram: u32,
}

struct TaskLoc {
    flash: MemorySection,
    ram: MemorySection,
}

impl TaskLoc {
    fn memory_linker_script(&self, stack_size: u32) -> String {
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
