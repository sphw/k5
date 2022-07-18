use crate::{Task, TCB};

pub(crate) fn start_root_task(_tcb: &TCB) -> ! {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(100));
    }
}
pub(crate) fn init_tcb_stack(_task: &Task, _tcb: &mut TCB) {}

pub(crate) fn init_kernel<'k, 't>(tasks: &'t [crate::TaskDesc]) -> &'k mut crate::Kernel {
    unimplemented!()
}

pub fn log(bytes: &[u8]) {}
#[derive(Default)]
pub struct SavedThreadState {}

impl SavedThreadState {
    pub fn set_syscall_return(&mut self, _ret: abi::SyscallReturn) {}
}
