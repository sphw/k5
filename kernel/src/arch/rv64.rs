use abi::{SyscallArgs, SyscallIndex, ThreadRef};
use core::arch::asm;
use core::mem::{self, MaybeUninit};
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};
use riscv::register::mcause::{Exception, Interrupt, Trap};
use riscv::register::mstatus::MPP;

use crate::regions::Region;
pub(crate) use crate::task::Task;
use crate::task_ptr::{TaskPtr, TaskPtrMut};
use crate::tcb::Tcb;
use crate::{Kernel, RegionAttr};

static mut KERNEL_INIT: AtomicBool = AtomicBool::new(false);
static mut KERNEL: MaybeUninit<Kernel> = MaybeUninit::uninit();

pub(crate) fn start_root_task(_task: &Task, tcb: &Tcb) -> ! {
    unsafe {
        set_current_tcb(tcb);
    }

    riscv::register::mepc::write(tcb.entrypoint);
    unsafe { riscv::register::mstatus::set_mpp(MPP::User) };
    unsafe {
        asm!(
            "
            csrrw a0, mscratch, a0
            sd sp,  32*8(a0)
            csrrw a0, mscratch, a0
            ld sp, ({sp})
            mret
            ",
            sp = in(reg) &tcb.stack_pointer,
            options(noreturn)
        )
    };
}

pub(crate) fn init_tcb_stack(task: &Task, tcb: &mut Tcb) {
    tcb.saved_state.sp = tcb.stack_pointer as u64;
    tcb.saved_state.pc = tcb.entrypoint as u64;
}

pub(crate) fn init_kernel<'k, 't>(tasks: &'t [crate::TaskDesc]) -> &'k mut crate::Kernel {
    log(b"LOG_START");
    unsafe {
        if KERNEL_INIT.load(Ordering::SeqCst) {
            panic!("kernel already inited");
        }
        let kern = KERNEL.write(Kernel::from_tasks(tasks).unwrap());
        KERNEL_INIT.store(true, Ordering::SeqCst);
        kern
    }
}

#[inline]
pub(crate) unsafe fn kernel() -> *mut Kernel {
    KERNEL.as_mut_ptr()
}

pub fn log(bytes: &[u8]) {
    extern "Rust" {
        fn board_log(bytes: &[u8]);
    }
    unsafe { board_log(bytes) };
}

#[derive(Default)]
#[repr(C)]
pub struct SavedThreadState {
    ra: u64,
    sp: u64,
    gp: u64,
    tp: u64,
    t0: u64,
    t1: u64,
    t2: u64,
    s0: u64,
    s1: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
    a7: u64,
    s2: u64,
    s3: u64,
    s4: u64,
    s5: u64,
    s6: u64,
    s7: u64,
    s8: u64,
    s9: u64,
    s10: u64,
    s11: u64,
    t3: u64,
    t4: u64,
    t5: u64,
    t6: u64,
    pc: u64,
    // Extra register to contain the last machine-mode stack pointer
    mpc: u64,
}

impl SavedThreadState {
    pub(super) fn syscall_args(&self) -> &SyscallArgs {
        // Safety: repr(c) guarentees the order of fields, we are taking the first
        // 6 fields as SyscallArgs
        unsafe { mem::transmute(&self.a2) }
    }

    pub fn syscall_args_mut(&mut self) -> &mut SyscallArgs {
        // Safety: repr(c) guarentees the order of fields, we are taking the first
        // 6 fields as SyscallArgs
        unsafe { mem::transmute(&mut self.a2) }
    }

    pub fn set_syscall_return(&mut self, ret: abi::SyscallReturn) {
        self.a0 = ret.into();
    }
}

pub(crate) fn translate_task_ptr<'a, T: ptr::Pointee + ?Sized>(
    task_ptr: TaskPtr<'a, T>,
    task: &Task,
) -> Option<&'a T> {
    // Safety: We only use return this reference when validated, so this is safe
    let r = unsafe { task_ptr.ptr() };
    let (ptr, _) = (r as *const T).to_raw_parts();
    validate_addr(ptr.addr(), mem::size_of_val(r), &task.region_table.regions).then_some(r)
}

pub(crate) fn translate_mut_task_ptr<'a, T: ptr::Pointee + ?Sized>(
    task_ptr: TaskPtrMut<'a, T>,
    task: &Task,
) -> Option<&'a mut T> {
    // Safety: We only use return this reference when validated, so this is safe
    let r = unsafe { task_ptr.ptr() };
    let (ptr, _) = (r as *mut T).to_raw_parts();
    validate_addr(ptr.addr(), mem::size_of_val(r), &task.region_table.regions).then_some(r)
}

fn validate_addr(addr: usize, len: usize, regions: &[Region]) -> bool {
    let end = addr + len - 1;
    regions.iter().any(|r| {
        r.range.contains(&addr) && r.range.contains(&end) && r.attr.contains(RegionAttr::Read)
    })
}

pub(crate) fn clear_mem(_task: &Task) {}

pub(crate) unsafe fn set_current_tcb(task: &Tcb) {
    riscv::register::mscratch::write((task as *const Tcb).addr())
}

pub(super) unsafe fn get_current_tcb() -> &'static mut Tcb {
    &mut *(riscv::register::mscratch::read() as *mut Tcb)
}

unsafe fn trap_handler(index: SyscallIndex) {
    let cause = riscv::register::mcause::read();
    match cause.cause() {
        Trap::Interrupt(Interrupt::MachineExternal) => {}
        Trap::Exception(Exception::UserEnvCall) => {
            let tcb = get_current_tcb();
            let args = unsafe { tcb.saved_state.syscall_args() };
            tcb.saved_state.pc += 4;
            super::syscall_inner(index);
        }
        _ => {}
    }
}

#[inline]
pub(crate) fn switch_thread(kernel: &mut Kernel, tcb_ref: ThreadRef) {
    let current_tcb = unsafe { get_current_tcb() };
    let tcb = kernel.scheduler.get_tcb_mut(tcb_ref).unwrap();
    tcb.saved_state.mpc = current_tcb.saved_state.mpc;
    let task = tcb.task;
    // Safety: The TCB comes from the kernel which is stored statically so this is safe
    unsafe { set_current_tcb(tcb) }
    let task = kernel.task(task).unwrap();
    //apply_region_table(&task.region_table);
}

#[no_mangle]
#[export_name = "_start_trap"]
#[naked]
unsafe extern "C" fn _start_trap() -> ! {
    asm!(
        "
         .align 4
         # we store the current task pointer in mscratch
         # so we swap it into a0, and then save all the pointers to saved state
         csrrw a0, mscratch, a0
         sd ra,   0*8(a0)
         sd sp,   1*8(a0)
         sd gp,   2*8(a0)
         sd tp,   3*8(a0)
         sd t0,   4*8(a0)
         sd t1,   5*8(a0)
         sd t2,   6*8(a0)
         sd s0,   7*8(a0)
         sd s1,   8*8(a0)
         # sd a0,  9*8(a0) # skipping a0 because we are using it to store current TCB
         sd a1,  10*8(a0)
         sd a2,  11*8(a0)
         sd a3,  12*8(a0)
         sd a4,  13*8(a0)
         sd a5,  14*8(a0)
         sd a6,  15*8(a0)
         sd a7,  16*8(a0)
         sd s2,  17*8(a0)
         sd s3,  18*8(a0)
         sd s4,  19*8(a0)
         sd s5,  20*8(a0)
         sd s6,  21*8(a0)
         sd s7,  22*8(a0)
         sd s8,  23*8(a0)
         sd s9,  24*8(a0)
         sd s10, 25*8(a0)
         sd s11, 26*8(a0)
         sd t3,  27*8(a0)
         sd t4,  28*8(a0)
         sd t5,  29*8(a0)
         sd t6,  30*8(a0)

         csrr a1, mepc # store task pc
         sd a1,  31*8(a0)

         csrr a1, mscratch
         sd a1, 9*8(a0) # store a0 now that

         ld sp, 32*8(a0) # load old machine stack pointer
         # TODO(sphw): swap back mscratch
         csrrw a0, mscratch, a0


         jal {trap_handler}

         csrrw t6, mscratch, t6
         ld t5,  31*8(t6)     # restore mepc
         csrw mepc, t5

         ld t5,  31*8(t6)
         ld ra,   0*8(t6)
         ld gp,   2*8(t6)
         ld tp,   3*8(t6)
         ld t0,   4*8(t6)
         ld t1,   5*8(t6)
         ld t2,   6*8(t6)
         ld s0,   7*8(t6)
         ld s1,   8*8(t6)
         ld a0,   9*8(t6)
         ld a1,  10*8(t6)
         ld a2,  11*8(t6)
         ld a3,  12*8(t6)
         ld a4,  13*8(t6)
         ld a5,  14*8(t6)
         ld a6,  15*8(t6)
         ld a7,  16*8(t6)
         ld s2,  17*8(t6)
         ld s3,  18*8(t6)
         ld s4,  19*8(t6)
         ld s5,  20*8(t6)
         ld s6,  21*8(t6)
         ld s7,  22*8(t6)
         ld s8,  23*8(t6)
         ld s9,  24*8(t6)
         ld s10, 25*8(t6)
         ld s11, 26*8(t6)
         ld t3,  27*8(t6)
         ld t4,  28*8(t6)
         ld t5,  29*8(t6)
         sd sp,  32*8(t6)
         ld sp,   1*8(t6)

         csrrw t6, mscratch, t6 #
         csrrw t5, mscratch, t5
         ld t6,  30*8(t5)
         csrrw t5, mscratch, t5

         mret

     ",
     trap_handler = sym trap_handler,
     options(noreturn)
    )
}
