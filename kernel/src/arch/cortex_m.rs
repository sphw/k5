use core::arch::asm;
use core::mem::MaybeUninit;
use core::{
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

use crate::{task_ptr::TaskPtrMut, Kernel, Task, TaskDesc, TCB};

const INITIAL_PSR: u32 = 1 << 24;
const EXC_RETURN: u32 = 0xFFFFFFED; //FIXME(sphw): this is only correct on v8m in secure mode

static mut KERNEL: MaybeUninit<Kernel> = MaybeUninit::uninit();
static mut CURRENT_TCB: AtomicPtr<TCB> = AtomicPtr::new(ptr::null_mut());

pub unsafe fn init_kernel(tasks: &[TaskDesc], idle_index: usize) -> &mut Kernel {
    KERNEL.write(Kernel::from_tasks(tasks, idle_index).unwrap())
}

#[inline]
unsafe fn kernel() -> *mut Kernel {
    KERNEL.as_mut_ptr()
}

#[inline]
unsafe fn current_tcb() -> *mut TCB {
    CURRENT_TCB.load(Ordering::SeqCst)
}

unsafe fn set_current_tcb(task: &TCB) {
    CURRENT_TCB.store(core::mem::transmute(task), Ordering::SeqCst);
}

pub fn start_root_task(task: &TCB) -> ! {
    unsafe {
        set_current_tcb(task);
    }

    unsafe {
        cortex_m::register::psp::write(task.stack_pointer as u32);
    }
    // The goal here is to jump to our task in unprivelleged mode,
    // but for ARM requires you to be in Handler mode to switch the privillege level
    // of execution. So we call a sys call with a specific argument, that is handled as a
    // special case in `SVCall`. I don't really like this technique, since it increases the cycle
    // count (and branching) of the syscall handler. This is what FreeRTOS and Hubris both do
    // though, but I'd love to change it.
    unsafe { asm!("svc #0xFF", options(noreturn)) }
}

pub fn init_tcb_stack(task: &Task, tcb: &mut TCB) {
    let mut stack_ptr: TaskPtrMut<ExceptionFrame> =
        unsafe { TaskPtrMut::from_raw_parts(tcb.stack_pointer, ()) };
    let stack_exc_frame = task
        .validate_mut_ptr(&mut stack_ptr)
        .expect("stack pointer not in task memory");
    *stack_exc_frame = ExceptionFrame::default();
    stack_exc_frame.pc = (tcb.entrypoint | 1) as u32;
    stack_exc_frame.xpsr = INITIAL_PSR;
    stack_exc_frame.lr = 0xFFFF_FFFF;
}

#[derive(Default)]
pub struct SavedThreadState {}

#[repr(C)]
#[derive(Default)]
pub struct ExceptionFrame {
    r0: u32,
    r1: u32,
    r2: u32,
    r3: u32,
    r12: u32,
    lr: u32,
    pc: u32,
    xpsr: u32,
    fpu_regs: [u32; 16], //TODO: exclude these for non-FPU targets
    fpscr: u32,
    reserved: u32,
}

#[allow(non_snake_case)]
#[naked]
#[no_mangle]
pub unsafe extern "C" fn SVCall() {
    asm!(
        "
        mov r0, lr
        mov r1, #0xFFFFFFF3
        bic r0, r1
        cmp r0, #0x8
        beq 1f
        @ TODO(sphw): implement normal syscall convention
        1:
        movs r0, #1
        msr CONTROL, r0
        mov lr, {exc_return}
        bx lr
        ",
        exc_return = const EXC_RETURN,
        options(noreturn)
    )
}
