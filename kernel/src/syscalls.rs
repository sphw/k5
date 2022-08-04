use core::mem::{self, MaybeUninit};

use abi::{
    Cap, CapListEntry, CapRef, RecvResp, SyscallArgs, SyscallDataType, SyscallReturn,
    SyscallReturnType, ThreadRef,
};
use defmt::{error, Format};

use crate::{
    regions::Region,
    task::TaskState,
    task_ptr::{TaskPtr, TaskPtrMut},
    tcb::{RecvReq, RecvReqInner, RecvRes, Tcb},
    CapEntry, DomainEntry, IPCMsgBody, Kernel, KernelError, RegionAttr,
};

#[repr(C)]
pub(crate) struct SendCall {
    buf_addr: usize,
    buf_len: usize,
    cap_ref: CapRef,
}

/// # Safety
/// This trait is safe to implement as long as the user guarentees that `Self` and `SyscallArgs`, have
/// the less than or equal size and alignment.
pub(crate) unsafe trait SysCall {
    #[inline]
    fn from_args(args: &SyscallArgs) -> &Self
    where
        Self: Sized,
    {
        // Safety: this trait will only be implemented for types where this is safe
        unsafe { mem::transmute(args) }
    }
    fn exec(&self, arg_type: SyscallDataType, kern: &mut Kernel)
        -> Result<CallReturn, KernelError>;
}

// Safety: The only requirement for safety in this trait is that the implementer has the same alignment and less than or equal length as [`SyscallArgs`]
unsafe impl SysCall for SendCall {
    #[inline]
    fn exec(
        &self,
        arg_type: SyscallDataType,
        kern: &mut Kernel,
    ) -> Result<CallReturn, KernelError> {
        let msg = get_msg(kern, arg_type, self.buf_addr, self.buf_len)?;
        kern.send(self.cap_ref, msg)?;
        let tcb = kern.scheduler.current_thread()?;
        let priority = tcb.priority;
        let next_thread = kern.scheduler.next_thread(priority);
        Ok(match next_thread {
            Some(next_thread) => CallReturn::Switch {
                next_thread: next_thread.tcb_ref,
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
        let msg = get_msg(kern, arg_type, self.in_addr, self.in_len)?;
        let out_buf =
        // Safety: the caller is giving over memory to us, to overwrite
        // TaskPtrMut ensures that the memory belongs to the correct task
            unsafe { TaskPtrMut::<'_, [u8]>::from_raw_parts(self.out_addr, self.out_len) };
        let recv_resp =
        // Safety: the caller is giving over memory to us, to overwrite
        // TaskPtrMut ensures that the memory belongs to the correct task
            unsafe { TaskPtrMut::<'_, MaybeUninit<RecvResp>>::from_raw_parts(self.resp_addr, ()) };
        let next_thread = kern.call(
            self.cap_ref,
            msg,
            RecvReq {
                mask: 0, // NOTE: this is replaced by the endpoints addr in `call`
                resp: recv_resp,
                inner: RecvReqInner::Buf { out: out_buf },
            },
        )?;
        Ok(CallReturn::Switch {
            next_thread,
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
        let recv_req_inner = if arg_type == SyscallDataType::Page {
            RecvReqInner::Page
        } else {
            let out_buf =
        // Safety: the caller is giving over memory to us, to overwrite
        // TaskPtrMut ensures that the memory belongs to the correct task
            unsafe { TaskPtrMut::<'_, [u8]>::from_raw_parts(self.out_addr, self.out_len as usize) };
            RecvReqInner::Buf { out: out_buf }
        };
        let recv_resp =
        // Safety: the caller is giving over memory to us, to overwrite
        // TaskPtrMut ensures that the memory belongs to the correct task
            unsafe { TaskPtrMut::<'_, MaybeUninit<RecvResp>>::from_raw_parts(self.resp_addr, ()) };
        let recv_req = RecvReq {
            mask: self.mask,
            resp: recv_resp,
            inner: recv_req_inner,
        };
        let tcb = kern.scheduler.current_thread_mut()?;
        let task = kern
            .tasks
            .get_mut(tcb.task.0)
            .ok_or(KernelError::InvalidTaskRef)?;
        if let RecvRes::NotFound(req) = tcb.recv(task, recv_req)? {
            Ok(CallReturn::Replace {
                next_thread: kern.scheduler.wait(req, false)?,
            })
        } else {
            defmt::println!("got msg in recv");
            Ok(CallReturn::Return {
                ret: abi::SyscallReturn::new()
                    .with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
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
        // Safety: the caller is giving over memory to us, to overwrite
        // TaskPtrMut ensures that the memory belongs to the correct task
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
pub(crate) struct PanikCall {
    addr: usize,
    len: usize,
}

// Safety: The only requirement for safety in this trait is that the implementer has the same alignment and less than or equal length as [`SyscallArgs`]
unsafe impl SysCall for PanikCall {
    fn exec(
        &self,
        _arg_type: SyscallDataType,
        kern: &mut Kernel,
    ) -> Result<CallReturn, KernelError> {
        let tcb = kern.scheduler.current_thread()?;
        let task_ref = tcb.task;
        let buf = get_buf::<512>(kern, tcb, self.addr, self.len)?;
        if let Ok(s) = core::str::from_utf8(buf) {
            error!("task {:?} paniked: {}", task_ref.0, s);
        } else {
            error!("task {:?} paniked with invalid msg", task_ref.0);
        }
        kern.scheduler.wait_queue.retain(|e| {
            if let Some(tcb) = kern.scheduler.tcbs.get(*e.tcb_ref) {
                tcb.task != task_ref
            } else {
                false
            }
        });
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
        let (priority, budget, cooldown, caps) =
            if let Some(priority) = priority
                && let Some(budget) = budget
                && let Some(cooldown) = cooldown
                && let Some(caps) =  caps {
                    (priority, budget, cooldown, caps)
                }else {
                    return Err(KernelError:: InitTCBNotFound);
                };
        kern.spawn_thread(task_ref, priority, budget, cooldown, task.entrypoint, caps)?;
        let next_thread = kern
            .scheduler
            .next_thread(0)
            .unwrap_or_else(DomainEntry::idle);
        let next_thread = kern.scheduler.switch_thread(next_thread)?;
        Ok(CallReturn::Replace { next_thread })
    }
}

#[repr(C)]
pub(crate) struct ListenCall {
    cap_ref: CapRef,
}

// Safety: The only requirement for safety in this trait is that the implementer has the same alignment and less than or equal length as [`SyscallArgs`]
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

// Safety: The only requirement for safety in this trait is that the implementer has the same alignment and less than or equal length as [`SyscallArgs`]
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

fn get_msg(
    kern: &mut Kernel,
    arg_type: SyscallDataType,
    addr: usize,
    len: usize,
) -> Result<IPCMsgBody, KernelError> {
    let tcb = kern.scheduler.current_thread()?;
    match arg_type {
        SyscallDataType::Short => todo!(),
        SyscallDataType::Copy => {
            let slice = get_buf::<1024>(kern, tcb, addr, len)?;
            Ok(IPCMsgBody::Buf(alloc::boxed::Box::from(slice)))
        }
        SyscallDataType::Page => {
            let task = kern
                .tasks
                .get_mut(tcb.task.0)
                .ok_or(KernelError::InvalidTaskRef)?;
            // Safety: the caller is giving over memory to us, to overwrite
            // TaskPtrMut ensures that the memory belongs to the correct task
            let slice = unsafe { TaskPtrMut::<'_, [u8]>::from_raw_parts(addr, len) };
            let slice = if let Some(slice) = task.validate_mut_ptr(slice) {
                slice
            } else {
                return Err(KernelError::ABI(abi::Error::BadAccess));
            };
            task.region_table.pop(Region {
                range: addr..addr + len,
                attr: RegionAttr::Write | RegionAttr::Read | RegionAttr::Exec,
            });
            Ok(IPCMsgBody::Page(slice))
        }
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
    Ok(slice)
}

#[derive(Format)]
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
