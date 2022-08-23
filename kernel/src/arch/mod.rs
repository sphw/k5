#[cfg(feature = "cortex_m")]
pub mod cortex_m;
#[cfg(feature = "std")]
pub mod dummy;
#[cfg(feature = "rv64")]
pub mod rv64;

use crate::{syscalls::CallReturn, KernelError};

#[cfg(feature = "cortex_m")]
pub use self::cortex_m::*;

#[cfg(feature = "rv64")]
pub use self::rv64::*;

use abi::{SyscallIndex, SyscallReturn, SyscallReturnType};
#[cfg(feature = "std")]
pub use dummy::*;

pub(crate) fn syscall_inner(index: SyscallIndex) {
    // Safety: We are safe to access global state due to our interrupt model
    let args = unsafe {
        let tcb = get_current_tcb();
        tcb.saved_state.syscall_args()
    };
    // Safety: We are safe to access global state due to our interrupt model
    let kernel = unsafe { &mut *kernel() };
    let ret = match kernel.syscall(index, args) {
        Ok(ret) => ret,
        Err(KernelError::ABI(err)) => CallReturn::Return {
            ret: SyscallReturn::new()
                .with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Error)
                .with(SyscallReturn::SYSCALL_LEN, u8::from(err) as u64),
        },
        err => {
            let _ = err.unwrap();
            return;
        }
    };
    match ret {
        CallReturn::Replace { next_thread } => switch_thread(kernel, next_thread),
        CallReturn::Switch { next_thread, ret } => {
            // Safety: `Switch` guarentees that the current TCB has been left in place
            // and not killed or deleted. Meaning that `CURRENT_TCB` contains a
            // valid pointer. Also since the kernel is single threaded, we
            // are guareneteed to be able to safely access `CURRENT_TCB`
            let tcb = unsafe { get_current_tcb() };
            tcb.saved_state.set_syscall_return(ret);
            switch_thread(kernel, next_thread)
        }
        CallReturn::Return { ret } => {
            // Safety: `Switch` guarentees that the current TCB has been left in place
            // and not killed or deleted. Meaning that `CURRENT_TCB` contains a
            // valid pointer. Also since the kernel is single threaded, we
            // are guareneteed to be able to safely access `CURRENT_TCB`
            let tcb = unsafe { get_current_tcb() };
            tcb.saved_state.set_syscall_return(ret);
        }
    }
}
