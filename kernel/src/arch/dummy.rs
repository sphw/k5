use crate::task_ptr::{TaskPtr, TaskPtrMut};
use crate::{Task, TCB};

pub(crate) fn start_root_task(_tcb: &TCB) -> ! {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(100));
    }
}
pub(crate) fn init_tcb_stack(_task: &Task, _tcb: &mut TCB) {}

pub(crate) fn init_kernel<'k, 't>(tasks: &'t [crate::TaskDesc]) -> &'k mut crate::Kernel {
    let _ = crate::Kernel::from_tasks(tasks).unwrap();
    unimplemented!()
}

pub fn log(_bytes: &[u8]) {}
#[derive(Default)]
pub struct SavedThreadState {}

impl SavedThreadState {
    pub fn set_syscall_return(&mut self, _ret: abi::SyscallReturn) {}
}

pub(crate) fn translate_task_ptr<'a, T: std::ptr::Pointee + ?Sized>(
    task_ptr: TaskPtr<'a, T>,
    _task: &Task,
) -> Option<&'a T> {
    // Safety: This code is like radioactively bad and is just straight up a terrible idea
    // A jedi wouldn't teach you these tricks for a good reason. Just don't do this
    // `dummy` is only used in tests, and so this is fine.....
    unsafe {
        let r = task_ptr.ptr();
        let (_, metadata) = (r as *const T).to_raw_parts();
        let vec = vec![0u8; std::mem::size_of_val(r)];
        let b: Box<[u8]> = vec.into();
        let addr = Box::leak(b).as_ptr().addr();
        Some(&*core::ptr::from_raw_parts(addr as *const (), metadata))
    }
}

pub(crate) fn translate_mut_task_ptr<'a, T: std::ptr::Pointee + ?Sized>(
    task_ptr: TaskPtrMut<'a, T>,
    _task: &Task,
) -> Option<&'a mut T> {
    // Safety: This code is like radioactively bad and is just straight up a terrible idea
    // A jedi wouldn't teach you these tricks for a good reason. Just don't do this
    // `dummy` is only used in tests, and so this is fine.....
    unsafe {
        let r = task_ptr.ptr();
        let (_, metadata) = (r as *mut T).to_raw_parts();
        // YOLO LEAK SOME MEMORY WOO
        let vec = vec![0u8; std::mem::size_of_val(r)];
        let b: Box<[u8]> = vec.into();
        let addr = Box::leak(b).as_ptr().addr();
        Some(&mut *core::ptr::from_raw_parts_mut(
            addr as *mut (),
            metadata,
        ))
    }
}
