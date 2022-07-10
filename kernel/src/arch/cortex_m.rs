use core::arch::asm;
use core::{
    mem, ptr,
    sync::atomic::{AtomicPtr, Ordering},
};
use mem::MaybeUninit;

use abi::{SyscallArgs, SyscallIndex, SyscallReturn};

use crate::{task_ptr::TaskPtrMut, Kernel, Task, TaskDesc, TCB};

const INITIAL_PSR: u32 = 1 << 24;
const EXC_RETURN: u32 = 0xFFFFFFED; //FIXME(sphw): this is only correct on v8m in secure mode

static mut KERNEL: MaybeUninit<Kernel> = MaybeUninit::uninit();
#[no_mangle]
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
    CURRENT_TCB.store(mem::transmute(task), Ordering::SeqCst);
}

pub fn start_root_task(task: &TCB) -> ! {
    unsafe {
        set_current_tcb(task);
    }

    unsafe {
        cortex_m::register::psp::write(task.saved_state.psp as u32);
    }
    // The goal here is to jump to our task in unprivelleged mode,
    // but for ARM requires you to be in Handler mode to switch the privillege level
    // of execution. So we call a sys call with a specific argument, that is handled as a
    // special case in `SVCall`. I don't really like this technique, since it increases the cycle
    // count (and branching) of the syscall handler. This is what FreeRTOS and Hubris both do
    // though, but I'd love to change it.
    unsafe {
        asm!(
            "
            ldm {state}, {{r4-r11}}
            svc #0xFF
            ",
            state = in(reg) &task.saved_state.r4,
            options(noreturn)
        )
    }
}

pub fn init_tcb_stack(task: &Task, tcb: &mut TCB) {
    let stack_addr = tcb.stack_pointer - mem::size_of::<ExceptionFrame>();
    let mut stack_ptr: TaskPtrMut<ExceptionFrame> =
        unsafe { TaskPtrMut::from_raw_parts(stack_addr, ()) };
    let stack_exc_frame = task
        .validate_mut_ptr(&mut stack_ptr)
        .expect("stack pointer not in task memory");
    *stack_exc_frame = ExceptionFrame::default();
    stack_exc_frame.pc = (tcb.entrypoint | 1) as u32;
    stack_exc_frame.xpsr = INITIAL_PSR;
    stack_exc_frame.lr = 0xFFFF_FFFF;
    tcb.saved_state.psp = stack_addr as u32;
    tcb.saved_state.exc_return = EXC_RETURN;
}

#[repr(C)]
#[derive(Default)]
pub struct SavedThreadState {
    r4: u32,
    r5: u32,
    r6: u32,
    r7: u32,
    r8: u32,
    r9: u32,
    r10: u32,
    r11: u32,
    psp: u32,
    exc_return: u32,
    s16: u32,
    s17: u32,
    s18: u32,
    s19: u32,
    s20: u32,
    s21: u32,
    s22: u32,
    s23: u32,
    s24: u32,
    s25: u32,
    s26: u32,
    s27: u32,
    s28: u32,
    s29: u32,
    s30: u32,
    s31: u32,
}

impl SavedThreadState {
    fn syscall_args(&self) -> &SyscallArgs {
        // Safety: repr(c) guarentees the order of fields, we are taking the first
        // 6 fields as SyscallArgs
        unsafe { mem::transmute(self) }
    }

    fn syscall_args_mut(&mut self) -> &mut SyscallArgs {
        // Safety: repr(c) guarentees the order of fields, we are taking the first
        // 6 fields as SyscallArgs
        unsafe { mem::transmute(self) }
    }
}

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
        beq 1f @ jump to first task handler
        @ standard syscall convention
        movw r0, #:lower16:CURRENT_TCB
        movt r0, #:upper16:CURRENT_TCB
        ldr r1, [r0] @ load the value of CURRENT_TCB into r1
        movs r2, r1
        mrs r12, PSP @ store PSP in r12
        stm r2!, {{r4-r12, lr}} @ store r4-r11 & psp in r12
        vstm r2, {{s16-s31}} @ store float registers
        movs r0, r11 @ syscall index is stored in r11
        bl {inner}
        movw r0, #:lower16:CURRENT_TCB
        movt r0, #:upper16:CURRENT_TCB
        ldr r0, [r0]
        @ restore volatile registers, plus load PSP into r12
        ldm r0!, {{r4-r12, lr}}
        vldm r0, {{s16-s31}}
        msr PSP, r12

        bx lr

        1:
        movs r0, #1
        msr CONTROL, r0
        mov lr, {exc_return}
        bx lr
        ",
        inner = sym syscall_inner,
        exc_return = const EXC_RETURN,
        options(noreturn)
    )
}

#[allow(non_snake_case)]
#[naked]
#[no_mangle]
pub unsafe extern "C" fn SysTick() {
    asm!(
        " movw r0, #:lower16:CURRENT_TCB
        movt r0, #:upper16:CURRENT_TCB
        ldr r1, [r0] @ load the value of CURRENT_TCB into r1
        movs r2, r1
        mrs r12, PSP @ store PSP in r12
        stm r2!, {{r4-r12, lr}} @ store r4-r11 & psp in r12
        vstm r2, {{s16-s31}} @ store float registers
        bl {inner}
        movw r0, #:lower16:CURRENT_TCB
        movt r0, #:upper16:CURRENT_TCB
        ldr r0, [r0]
        @ restore volatile registers, plus load PSP into r12
        ldm r0!, {{r4-r12, lr}}
        vldm r0, {{s16-s31}}
        msr PSP, r12
        ",
        inner = sym systick_inner,
        options(noreturn)
    )
}

fn systick_inner() {
    let kernel = unsafe { &mut *kernel() };
    if let Some(tcb_ref) = kernel.scheduler.tick().unwrap() {
        unsafe { set_current_tcb(kernel.scheduler.get_tcb(tcb_ref).unwrap()) }
    }
}

fn syscall_inner(index: SyscallIndex) -> SyscallReturn {
    // TODO(sphw): figure out how safe this is,
    // do we need to copy
    let args = unsafe {
        let tcb = &*CURRENT_TCB.load(Ordering::SeqCst);
        tcb.saved_state.syscall_args()
    };
    let kernel = unsafe { &mut *kernel() };
    let (next_tcb, ret) = kernel.syscall(index, args).unwrap();
    let args = unsafe {
        let tcb = &mut *CURRENT_TCB.load(Ordering::SeqCst);
        tcb.saved_state.syscall_args_mut()
    };
    let (a1, a2) = ret.split();
    args.arg1 = a2 as usize;
    args.arg2 = a1 as usize;
    if let Some(tcb_ref) = next_tcb {
        unsafe { set_current_tcb(kernel.scheduler.get_tcb(tcb_ref).unwrap()) }
    }
    ret
}
