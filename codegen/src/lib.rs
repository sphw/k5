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
    pub entrypoint: usize,
    pub stack_space: Range<usize>,
    pub init_stack_size: usize,
    pub regions: Vec<Range<usize>>,
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
regions: &{:?}
}},",
                task.name, task.entrypoint, task.stack_space, task.init_stack_size, task.regions,
            );
        }
        code += "];\n";

        for (i, task) in self.tasks.iter().enumerate() {
            code += &format!(
                "pub const TASK_{}_INDEX: usize = {};\n",
                task.name.to_uppercase(),
                i
            );
            code += &format!(
                "pub const {}: kernel::ThreadBuilder = unsafe {{ kernel::ThreadBuilder::new({}) }};\n",
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
