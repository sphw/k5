use core::mem::{self, MaybeUninit};

use abi::{
    Cap, CapListEntry, CapRef, RecvResp, SyscallArgs, SyscallDataType, SyscallReturn,
    SyscallReturnType, ThreadRef,
};
use defmt::error;

use crate::{
    task::TaskState,
    task_ptr::{TaskPtr, TaskPtrMut},
    tcb::Tcb,
    CapEntry, Kernel, KernelError,
};

#[repr(C)]
pub(crate) struct SendCall {
    buf_addr: usize,
    buf_len: usize,
    cap_ref: CapRef,
}

pub unsafe trait SysCall {
    #[inline]
    fn from_args(args: &SyscallArgs) -> &Self
    where
        Self: Sized,
    {
        unsafe { mem::transmute(args) }
    }
    fn exec(&self, arg_type: SyscallDataType, kern: &mut Kernel)
        -> Result<CallReturn, KernelError>;
}

// Safety: The only requirement for safety in this trait is that the implementer has the same alignment and less than or equal length as [`SyscallArgs`]
unsafe impl SysCall for SendCall {
    #[inline]
    fn from_args(args: &SyscallArgs) -> &Self {
        unsafe { mem::transmute(args) }
    }

    #[inline]
    fn exec(
        &self,
        arg_type: SyscallDataType,
        kern: &mut Kernel,
    ) -> Result<CallReturn, KernelError> {
        if arg_type == SyscallDataType::Page {
            todo!()
        }
        let tcb = kern.scheduler.current_thread()?;
        let slice = get_buf::<1024>(kern, tcb, self.buf_addr, self.buf_len)?;
        let msg = alloc::boxed::Box::from(slice);
        let priority = tcb.priority;
        kern.send(self.cap_ref, msg)?;
        let next_thread = kern.scheduler.next_thread(priority);
        Ok(match next_thread {
            Some(next_thread) => CallReturn::Switch {
                next_thread,
                ret: abi::SyscallReturn::new()
                    .with(abi::SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
            },
            None => CallReturn::Return {
                ret: abi::SyscallReturn::new()
                    .with(abi::SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
            },
        })
    }
}

#[repr(C)]
pub(crate) struct CallSysCall {
    in_addr: usize,
    in_len: usize,
    cap_ref: CapRef,
    resp_addr: usize,
    out_addr: usize,
    out_len: usize,
}

// Safety: The only requirement for safety in this trait is that the implementer has the same alignment and less than or equal length as [`SyscallArgs`]
unsafe impl SysCall for CallSysCall {
    #[inline]
    fn exec(
        &self,
        arg_type: SyscallDataType,
        kern: &mut Kernel,
    ) -> Result<CallReturn, KernelError> {
        if arg_type == SyscallDataType::Page {
            todo!()
        }
        let tcb = kern.scheduler.current_thread()?;
        let slice = get_buf::<1024>(kern, tcb, self.in_addr, self.in_len)?;
        let msg = alloc::boxed::Box::from(slice);
        // Safety: the caller is giving over memory to us, to overwrite
        // TaskPtrMut ensures that the memory belongs to the correct task
        let out_buf =
            unsafe { TaskPtrMut::<'_, [u8]>::from_raw_parts(self.out_addr, self.out_len) };
        // Safety: the caller is giving over memory to us, to overwrite
        // TaskPtrMut ensures that the memory belongs to the correct task
        let recv_resp =
            unsafe { TaskPtrMut::<'_, MaybeUninit<RecvResp>>::from_raw_parts(self.resp_addr, ()) };
        let next_thread = kern.call(self.cap_ref, msg, out_buf, recv_resp)?;
        Ok(CallReturn::Switch {
            next_thread: kern.scheduler.switch_thread(next_thread)?,
            ret: abi::SyscallReturn::new(),
        })
    }
}

#[repr(C)]
pub(crate) struct RecvCall {
    out_addr: usize,
    out_len: usize,
    mask: usize,
    resp_addr: usize,
}

// Safety: The only requirement for safety in this trait is that the implementer has the same alignment and less than or equal length as [`SyscallArgs`]
unsafe impl SysCall for RecvCall {
    fn exec(
        &self,
        arg_type: SyscallDataType,
        kern: &mut Kernel,
    ) -> Result<CallReturn, KernelError> {
        if arg_type == SyscallDataType::Page {
            todo!()
        }
        let out_buf =
        // Safety: the caller is giving over memory to us, to overwrite
        // TaskPtrMut ensures that the memory belongs to the correct task
            unsafe { TaskPtrMut::<'_, [u8]>::from_raw_parts(self.out_addr, self.out_len as usize) };
        // Safety: the caller is giving over memory to us, to overwrite
        // TaskPtrMut ensures that the memory belongs to the correct task
        let recv_resp =
            unsafe { TaskPtrMut::<'_, MaybeUninit<RecvResp>>::from_raw_parts(self.resp_addr, ()) };
        let tcb = kern.scheduler.current_thread_mut()?;
        let task = kern
            .tasks
            .get(tcb.task.0)
            .ok_or(KernelError::InvalidTaskRef)?;
        if !tcb.recv(task, self.mask, out_buf, recv_resp)? {
            // Safety: the caller is giving over memory to us, to overwrite
            // TaskPtrMut ensures that the memory belongs to the correct task
            let out_buf =
                unsafe { TaskPtrMut::<'_, [u8]>::from_raw_parts(self.out_addr, self.out_len) };
            // Safety: the caller is giving over memory to us, to overwrite
            // TaskPtrMut ensures that the memory belongs to the correct task
            let recv_resp = unsafe {
                TaskPtrMut::<'_, MaybeUninit<RecvResp>>::from_raw_parts(self.resp_addr, ())
            };
            Ok(CallReturn::Replace {
                next_thread: kern.scheduler.wait(self.mask, out_buf, recv_resp)?,
            })
        } else {
            Ok(CallReturn::Return {
                ret: abi::SyscallReturn::new(),
            })
        }
    }
}

#[repr(C)]
pub(crate) struct LogCall {
    in_addr: usize,
    in_len: usize,
}

// Safety: The only requirement for safety in this trait is that the implementer has the same alignment and less than or equal length as [`SyscallArgs`]
unsafe impl SysCall for LogCall {
    fn exec(
        &self,
        _arg_type: SyscallDataType,
        kern: &mut Kernel,
    ) -> Result<CallReturn, KernelError> {
        let tcb = kern.scheduler.current_thread()?;

        let log_buf = get_buf::<255>(kern, tcb, self.in_addr, self.in_len)?;
        crate::defmt_log::log(tcb.task.0 as u8 + 1, log_buf);
        Ok(CallReturn::Return {
            ret: abi::SyscallReturn::new(),
        })
    }
}

#[repr(C)]
pub(crate) struct CapsCall {
    out_addr: usize,
    out_len: usize,
}

// Safety: The only requirement for safety in this trait is that the implementer has the same alignment and less than or equal length as [`SyscallArgs`]
unsafe impl SysCall for CapsCall {
    fn exec(
        &self,
        _arg_type: SyscallDataType,
        kern: &mut Kernel,
    ) -> Result<CallReturn, KernelError> {
        let tcb = kern.scheduler.current_thread()?;
        let task = kern.task(tcb.task)?;
        let slice = unsafe {
            TaskPtrMut::<'_, [CapListEntry]>::from_raw_parts(self.out_addr, self.out_len)
        };
        let slice = task
            .validate_mut_ptr(slice)
            .ok_or(KernelError::InvalidTaskPtr)?;

        let len = slice.len().min(tcb.capabilities.len());
        for (i, entry) in tcb.capabilities.iter().take(len).enumerate() {
            slice[i] = abi::CapListEntry {
                cap_ref: CapRef((entry as *const CapEntry).addr()),
                desc: entry.cap.clone(),
            };
        }
        let ret = abi::SyscallReturn::new()
            .with(abi::SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy)
            .with(abi::SyscallReturn::SYSCALL_LEN, len as u64);
        Ok(CallReturn::Return { ret })
    }
}

#[repr(C)]
pub(crate) struct PanikCall {}

unsafe impl SysCall for PanikCall {
    fn exec(
        &self,
        _arg_type: SyscallDataType,
        kern: &mut Kernel,
    ) -> Result<CallReturn, KernelError> {
        let tcb = kern.scheduler.current_thread()?;
        let task_ref = tcb.task;
        error!("task {:?} paniked", task_ref.0);
        for domain in &mut kern.scheduler.domains {
            let mut cursor = domain.cursor_front_mut();
            cursor.move_prev();
            while let Some(entry) = { cursor.next() } {
                if entry
                    .tcb_ref
                    .and_then(|t| kern.scheduler.tcbs.get(*t))
                    .is_some()
                {
                    cursor.remove_current();
                }
            }
        }
        let task = kern
            .tasks
            .get_mut(task_ref.0)
            .ok_or(KernelError::InvalidTaskRef)?;
        task.state = TaskState::Pending;
        task.reset_stack_ptr();
        let task = kern
            .tasks
            .get(task_ref.0)
            .ok_or(KernelError::InvalidTaskRef)?;
        let mut priority = None;
        let mut budget = None;
        let mut cooldown = None;
        let mut caps = None;

        for i in 0..16 {
            if let Some(tcb) = kern.scheduler.tcbs.get(i) {
                if tcb.task == task_ref {
                    let tcb = kern.scheduler.tcbs.remove(i).unwrap();
                    if tcb.entrypoint == task.entrypoint.addr() {
                        priority = Some(tcb.priority);
                        budget = Some(tcb.budget);
                        cooldown = Some(tcb.cooldown);
                        caps = Some(tcb.capabilities);
                    }
                }
            }
        }
        let (priority, budget, cooldown, caps) = if let Some(priority) = priority && let Some(budget) = budget && let Some(cooldown) = cooldown && let Some(caps) =  caps {
                    (priority, budget, cooldown, caps)
                }else {
                    return Err(KernelError:: InitTCBNotFound);
                };
        kern.spawn_thread(task_ref, priority, budget, cooldown, task.entrypoint, caps)?;
        let next_thread = kern
            .scheduler
            .next_thread(0)
            .unwrap_or_else(ThreadRef::idle);
        let next_thread = kern.scheduler.switch_thread(next_thread)?;
        Ok(CallReturn::Replace { next_thread })
    }
}

#[repr(C)]
pub(crate) struct ListenCall {
    cap_ref: CapRef,
}

unsafe impl SysCall for ListenCall {
    fn exec(
        &self,
        _arg_type: SyscallDataType,
        kern: &mut Kernel,
    ) -> Result<CallReturn, KernelError> {
        let tcb = kern.scheduler.current_thread()?;
        let listen = match tcb.cap(self.cap_ref)? {
            abi::Cap::Listen(listen) => listen,
            _ => {
                return Err(KernelError::ABI(abi::Error::InvalidCap));
            }
        };
        kern.registry
            .listen(
                *listen,
                abi::Endpoint {
                    tcb_ref: kern.scheduler.current_thread.tcb_ref,
                    addr: 0,
                    disposable: false,
                },
            )
            .map_err(KernelError::ABI)?;
        Ok(CallReturn::Return {
            ret: SyscallReturn::new().with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
        })
    }
}

#[repr(C)]
pub(crate) struct ConnectCall {
    cap_ref: CapRef,
}

unsafe impl SysCall for ConnectCall {
    fn exec(
        &self,
        _arg_type: SyscallDataType,
        kern: &mut Kernel,
    ) -> Result<CallReturn, KernelError> {
        let tcb = kern.scheduler.current_thread_mut()?;
        let connect = match tcb.cap(self.cap_ref)? {
            abi::Cap::Connect(connect) => connect,
            _ => {
                return Err(KernelError::ABI(abi::Error::InvalidCap));
            }
        };
        let endpoint = kern.registry.connect(*connect).map_err(KernelError::ABI)?;
        tcb.add_cap(Cap::Endpoint(endpoint));
        let entry = tcb.capabilities.back().unwrap();
        let cap_ref = (entry.get_ref() as *const CapEntry).addr();
        Ok(CallReturn::Return {
            ret: SyscallReturn::new()
                .with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy)
                .with(SyscallReturn::SYSCALL_PTR, cap_ref as u64),
        })
    }
}

fn get_buf<'t, const N: usize>(
    kern: &Kernel,
    tcb: &'t Tcb,
    addr: usize,
    len: usize,
) -> Result<&'t [u8], KernelError> {
    let task = kern
        .tasks
        .get(tcb.task.0)
        .ok_or(KernelError::InvalidTaskRef)?;
    // Safety: the caller is giving over memory to us, to overwrite
    // TaskPtrMut ensures that the memory belongs to the correct task
    let slice = unsafe { TaskPtr::<'_, [u8]>::from_raw_parts(addr, len) };
    let slice = if let Some(buf) = task.validate_ptr(slice) {
        buf
    } else {
        return Err(KernelError::ABI(abi::Error::BadAccess));
    };
    if slice.len() > N {
        return Err(KernelError::ABI(abi::Error::BufferOverflow));
    }
    Ok(slice)
}

pub enum CallReturn {
    Replace {
        next_thread: ThreadRef,
    },
    Switch {
        next_thread: ThreadRef,
        ret: abi::SyscallReturn,
    },
    Return {
        ret: abi::SyscallReturn,
    },
}
