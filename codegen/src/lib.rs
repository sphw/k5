use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::ops::Range;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct TaskList {
    pub tasks: Vec<Task>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Task {
    pub name: String,
    pub entrypoint: u32,
    pub stack_space: Range<u32>,
    pub init_stack_size: u32,
    pub flash_region: Range<u32>,
    pub ram_region: Range<u32>,
}

impl TaskList {
    fn gen_code(&self) -> String {
        let mut code = format!(
            "
        pub static TASKS: &[kernel::TaskDesc] = &["
        );
        for task in &self.tasks {
            code += &format!(
                "kernel::TaskDesc {{
name: {:?},
entrypoint: {},
stack_space: {:?},
init_stack_size: {},
flash_region: {:?},
ram_region: {:?},
}},",
                task.name,
                task.entrypoint,
                task.stack_space,
                task.init_stack_size,
                task.flash_region,
                task.ram_region
            );
        }
        code += "];\n";

        for (i, task) in self.tasks.iter().enumerate() {
            code += &format!(
                "pub const TASK_{}_INDEX: usize = {};",
                task.name.to_uppercase(),
                i
            );
        }
        code
    }
}

pub fn gen_tasklist() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=K5_TASK_LIST");
    let env = env::var("K5_TASK_LIST")?;
    println!("cargo:rerun-if-changed={}", env);
    let task_list = fs::read(env)?;
    let task_list: TaskList = serde_json::from_slice(&task_list)?;
    let code = task_list.gen_code();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR environment variable not set"));
    fs::write(out_dir.join("codegen.rs"), code.as_bytes())?;
    Ok(())
}
