use core::arch::asm;
use core::ops::Range;
use core::sync::atomic::AtomicBool;
use core::{
    mem, ptr,
    sync::atomic::{AtomicPtr, Ordering},
};
use cortex_m::peripheral::scb::SystemHandler;
use mem::MaybeUninit;

use abi::{SyscallArgs, SyscallIndex, SyscallReturn};
use rtt_target::{rtt_init, UpChannel};

use crate::{
    task_ptr::{TaskPtr, TaskPtrMut},
    Kernel, Task, TaskDesc, TCB,
};

const INITIAL_PSR: u32 = 1 << 24;
const EXC_RETURN: u32 = 0xFFFFFFED; //FIXME(sphw): this is only correct on v8m in secure mode

static mut KERNEL_INIT: AtomicBool = AtomicBool::new(false);
static mut KERNEL: MaybeUninit<Kernel> = MaybeUninit::uninit();
#[no_mangle]
static mut CURRENT_TCB: AtomicPtr<TCB> = AtomicPtr::new(ptr::null_mut());

pub(crate) fn init_kernel<'k, 't>(tasks: &'t [TaskDesc]) -> &'k mut Kernel {
    // Safety: this is all unsafe due to the use of static mut, but its a kernel so watcha gonna do
    unsafe {
        if KERNEL_INIT.load(Ordering::SeqCst) {
            panic!("kernel already inited");
        }
        init_log();
        let kern = KERNEL.write(Kernel::from_tasks(tasks).unwrap());
        KERNEL_INIT.store(true, Ordering::SeqCst);
        kern
    }
}

#[inline]
unsafe fn kernel() -> *mut Kernel {
    KERNEL.as_mut_ptr()
}

unsafe fn set_current_tcb(task: &TCB) {
    CURRENT_TCB.store(task as *const TCB as *mut TCB, Ordering::SeqCst);
}

pub(crate) fn start_root_task(task: &TCB) -> ! {
    // Safety: start root ask is only called once when the kernel is initialized
    // This is marked unsafe, since it could allow an invalid pointer being saved to global state
    // TCB's locations are stable since they are stored in the `KERNEL` global, and we currently never remove them
    unsafe {
        set_current_tcb(task);
    }

    let mut p = cortex_m::Peripherals::take().unwrap();

    // set systick to lowest priority, so it won't interrupt the kernel
    //
    // Safety: this operation is basically safe, but cortex_m marks it as unsafe,
    // since if called in an interrupt handler it can lead to some bad side-effects
    // This whole function is only called once from `main`, so its safe
    unsafe {
        p.SCB.set_priority(SystemHandler::SysTick, 0xff);
    }

    let irq_count = (((p.ICB.ictr.read() & 0xF) + 1) * 32) as usize;
    // gets the irq count from icb's ictr register
    // ictr gives the count in blocks of 32, in the first 4 bytes.

    // Safety: this operation is "safe", because `start_root_task`,
    // is only ever run durring startup. Changing interrupt prioritys
    // CAN cause issues in criticial sections, but we aren't using those
    unsafe {
        for i in 0..irq_count {
            p.NVIC.ipr[i].write(0xFFu8);
        }
    }

    p.SYST.set_reload(400_000);
    p.SYST.clear_current();
    p.SYST.enable_counter();
    p.SYST.enable_interrupt();

    // Safety: we are currently in kernel mode, so setting the psp is safe
    unsafe {
        cortex_m::register::psp::write(task.saved_state.psp as u32);
    }

    // The goal here is to jump to our task in unprivelleged mode,
    // but for ARM requires you to be in Handler mode to switch the privillege level
    // of execution. So we call a sys call with a specific argument, that is handled as a
    // special case in `SVCall`. I don't really like this technique, since it increases the cycle
    // count (and branching) of the syscall handler. This is what FreeRTOS and Hubris both do
    // though, but I'd love to change it.
    //
    // Safety: ASM is always unsafe, but this is actually pretty "safe" as it just triggers a syscall
    unsafe {
        asm!("
            ldm {state}, {{r4-r11}}
            svc #0xFF
            ", state = in(reg) &task.saved_state.r4,
            options(noreturn)
        )
    }
}

pub(crate) fn init_tcb_stack(task: &Task, tcb: &mut TCB) {
    let stack_addr = tcb.stack_pointer - mem::size_of::<ExceptionFrame>();
    let stack_ptr: TaskPtrMut<ExceptionFrame> =
    // Safety: We are essentially inventing a lifetime here, but its fine because we are the
    // kernel and can guarantee that no one else will touch this memory until we say so
    // side note: I know this is ugly but clippy is being weird about the lint
        unsafe { TaskPtrMut::from_raw_parts(stack_addr, ()) };
    let stack_exc_frame = task
        .validate_mut_ptr(stack_ptr)
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

    pub fn syscall_args_mut(&mut self) -> &mut SyscallArgs {
        // Safety: repr(c) guarentees the order of fields, we are taking the first
        // 6 fields as SyscallArgs
        unsafe { mem::transmute(self) }
    }

    pub fn set_syscall_return(&mut self, ret: SyscallReturn) {
        let args = self.syscall_args_mut();
        let (a1, a2) = ret.split();
        args.arg1 = a2 as usize;
        args.arg2 = a1 as usize;
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
         bx lr
         ",
        inner = sym systick_inner,
        options(noreturn)
    )
}

fn systick_inner() {
    // Safety: This function is only ever called by the SysTick handler, which
    // can't preempt the kernel, so it is safe for us to access the kernel
    let kernel = unsafe { &mut *kernel() };
    if let Some(tcb_ref) = kernel.scheduler.tick().unwrap() {
        // Safety: The TCB comes from the kernel which is stored statically so this is safe
        unsafe { set_current_tcb(kernel.scheduler.get_tcb(tcb_ref).unwrap()) }
    }
}

fn syscall_inner(index: SyscallIndex) -> SyscallReturn {
    // Safety: We are safe to access global state due to our interrupt model
    let args = unsafe {
        let tcb = &*CURRENT_TCB.load(Ordering::SeqCst);
        tcb.saved_state.syscall_args()
    };
    // Safety: We are safe to access global state due to our interrupt model
    let kernel = unsafe { &mut *kernel() };
    let (next_tcb, ret) = kernel.syscall(index, args).unwrap();

    // Safety: We are safe to access global state due to our interrupt model,
    // plus we have thrown away the immutable reference to CURRENT_TCB above
    let tcb = unsafe { &mut *CURRENT_TCB.load(Ordering::SeqCst) };
    tcb.saved_state.set_syscall_return(ret);

    if let Some(tcb_ref) = next_tcb {
        // Safety: The TCB comes from the kernel which is stored statically so this is safe
        unsafe { set_current_tcb(kernel.scheduler.get_tcb(tcb_ref).unwrap()) }
    }
    ret
}

// RTT

static mut CHANNEL: Option<UpChannel> = None;

unsafe fn init_log() {
    let channels = rtt_init! {
            up: {
                0: {
                    size: 1024
                    mode: BlockIfFull
                    name: "defmt"
                }
            }

    };

    CHANNEL = Some(channels.up.0);
}

pub fn log(bytes: &[u8]) {
    // Safety: the kernel is non-reentrant so we can't get multiple mutable copies of `CHANNEL`
    if let Some(ch) = unsafe { &mut CHANNEL } {
        ch.write(bytes);
    }
}

pub(crate) fn translate_task_ptr<'a, T: ptr::Pointee + ?Sized>(
    task_ptr: TaskPtr<'a, T>,
    task: &Task,
) -> Option<&'a T> {
    // Safety: We only use return this reference when validated, so this is safe
    let r = unsafe { task_ptr.ptr() };
    let (ptr, _) = (r as *const T).to_raw_parts();
    validate_addr(
        ptr.addr(),
        mem::size_of_val(r),
        &task.ram_memory_region,
        &task.flash_memory_region,
    )
    .then(|| r)
}

pub(crate) fn translate_mut_task_ptr<'a, T: ptr::Pointee + ?Sized>(
    task_ptr: TaskPtrMut<'a, T>,
    task: &Task,
) -> Option<&'a mut T> {
    // Safety: We only use return this reference when validated, so this is safe
    let r = unsafe { task_ptr.ptr() };
    let (ptr, _) = (r as *mut T).to_raw_parts();
    validate_addr(
        ptr.addr(),
        mem::size_of_val(r),
        &task.ram_memory_region,
        &task.flash_memory_region,
    )
    .then(|| r)
}

fn validate_addr(
    addr: usize,
    len: usize,
    ram_range: &Range<usize>,
    flash_range: &Range<usize>,
) -> bool {
    let end = addr + len;
    (ram_range.contains(&addr) && ram_range.contains(&end))
        || (flash_range.contains(&addr) && flash_range.contains(&end))
}
