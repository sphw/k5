use crate::{Task, TCB};

pub fn start_root_task(_tcb: &TCB) -> ! {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(100));
    }
}
pub fn init_tcb_stack(_task: &Task, _tcb: &mut TCB) {}
#[derive(Default)]
pub struct SavedThreadState {}
