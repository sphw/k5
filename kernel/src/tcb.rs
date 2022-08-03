use core::mem::MaybeUninit;

use abi::{Cap, CapRef, Endpoint, RecvResp, SyscallReturn, SyscallReturnType};
use alloc::boxed::Box;
use cordyceps::{list::Links, List};

use crate::{
    arch, task_ptr::TaskPtrMut, CapEntry, IPCMsg, IPCMsgBody, KernelError, Task, TaskRef,
    ThreadState,
};

#[repr(C)]
pub(crate) struct Tcb {
    pub(crate) saved_state: arch::SavedThreadState,
    pub(crate) task: TaskRef, // Maybe use RC for this
    pub(crate) req_queue: List<IPCMsg>,
    pub(crate) state: ThreadState,
    pub(crate) priority: usize,
    pub(crate) budget: usize,
    pub(crate) cooldown: usize,
    pub(crate) capabilities: List<CapEntry>,
    pub(crate) stack_pointer: usize,
    pub(crate) entrypoint: usize,
    pub(crate) epoch: usize,
    pub(crate) rem_time: usize,
}

impl Tcb {
    // allowing too many args, because the alternative is ugly
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task: TaskRef,
        stack_pointer: usize,
        priority: usize,
        budget: usize,
        cooldown: usize,
        entrypoint: usize,
        epoch: usize,
        caps: List<CapEntry>,
    ) -> Self {
        Self {
            task,
            //_pad: 0,
            req_queue: List::new(),
            //reply_queue: List::new(),
            state: ThreadState::Ready,
            priority,
            budget,
            cooldown,
            capabilities: caps,
            stack_pointer,
            entrypoint,
            saved_state: Default::default(),
            epoch,
            rem_time: budget,
        }
    }

    #[inline]
    fn cap_entry(&self, cap_ref: CapRef) -> Result<&CapEntry, KernelError> {
        for c in self.capabilities.iter() {
            let c_addr = (c as *const CapEntry).addr();
            if c_addr == *cap_ref {
                return Ok(c);
            }
        }
        Err(KernelError::InvalidCapRef)
    }

    pub(crate) fn cap(&self, cap_ref: CapRef) -> Result<&Cap, KernelError> {
        self.cap_entry(cap_ref).map(|e| &e.cap)
    }

    pub(crate) fn endpoint(&mut self, cap_ref: CapRef) -> Result<Endpoint, KernelError> {
        let dest_cap = self.cap_entry(cap_ref)?;
        let endpoint = if let Cap::Endpoint(endpoint) = dest_cap.cap {
            endpoint
        } else {
            return Err(KernelError::ABI(abi::Error::InvalidCap));
        };
        if endpoint.disposable {
            // Safety: this function is marked as unsafe, because the user must guarentee that
            // the item is in the list. dest_cap comes from self.capabilities, so this is safe
            unsafe {
                self.capabilities.remove(dest_cap.into());
            }
        }
        Ok(endpoint)
    }

    pub(crate) fn add_cap(&mut self, cap: Cap) {
        self.capabilities.push_back(Box::pin(CapEntry {
            _links: Links::default(),
            cap,
        }));
    }

    pub(crate) fn recv<'r>(
        &mut self,
        task: &Task,
        req: RecvReq<'r>,
    ) -> Result<RecvRes<'r>, KernelError> {
        match self.recv_inner(task, req) {
            res @ Ok(RecvRes::Copy) => {
                self.saved_state.set_syscall_return(
                    SyscallReturn::new().with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
                );
                res
            }
            res @ Ok(RecvRes::Page) => {
                self.saved_state.set_syscall_return(
                    SyscallReturn::new().with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Page),
                );
                res
            }
            Err(KernelError::ABI(err)) => {
                self.saved_state.set_syscall_return(err.into());
                Ok(RecvRes::Page)
            }
            res => res,
        }
    }

    #[inline]
    fn recv_inner<'r>(
        &mut self,
        task: &Task,
        req: RecvReq<'r>,
    ) -> Result<RecvRes<'r>, KernelError> {
        let mut cursor = self.req_queue.cursor_front_mut();
        cursor.move_prev();
        let mut found = false;
        while let Some(msg) = {
            cursor.move_next();
            cursor.current()
        } {
            if msg.addr & req.mask == req.mask {
                found = true;
                break;
            }
        }
        if !found {
            return Ok(RecvRes::NotFound(req));
        }
        let msg = cursor.remove_current().unwrap();
        let (recv_res, mut resp) = match &msg.body {
            IPCMsgBody::Buf(buf) => {
                let out = if let RecvReqInner::Buf { out } = req.inner {
                    out
                } else {
                    return Err(abi::Error::ReturnTypeMismatch.into());
                };

                let out_buf = task.validate_mut_ptr(out).ok_or(abi::Error::BadAccess)?;
                if out_buf.len() != buf.len() {
                    return Err(abi::Error::ReturnTypeMismatch.into());
                }
                out_buf.copy_from_slice(buf);
                (
                    RecvRes::Copy,
                    RecvResp {
                        cap: None,
                        inner: abi::RecvRespInner::Copy(buf.len()),
                    },
                )
            }
            IPCMsgBody::Page(ptr) => (
                RecvRes::Page,
                RecvResp {
                    cap: None,
                    inner: abi::RecvRespInner::Page {
                        addr: ptr.addr(),
                        len: ptr.size(),
                    },
                },
            ),
        };

        if let Some(reply) = msg.reply_endpoint {
            self.add_cap(Cap::Endpoint(reply));
            let cap_ptr = &*self.capabilities.back().unwrap() as *const CapEntry;
            resp.cap = Some(CapRef(cap_ptr.addr()));
        }
        let recv_resp = task
            .validate_mut_ptr(req.resp)
            .ok_or(abi::Error::BadAccess)?;
        recv_resp.write(resp);
        Ok(recv_res)
    }
}

pub(crate) struct RecvReq<'a> {
    pub(crate) mask: usize,
    pub(crate) resp: TaskPtrMut<'a, MaybeUninit<RecvResp>>,
    pub(crate) inner: RecvReqInner<'a>,
}
pub(crate) enum RecvReqInner<'a> {
    Page,
    Buf { out: TaskPtrMut<'a, [u8]> },
}

pub(crate) enum RecvRes<'a> {
    Page,
    Copy,
    NotFound(RecvReq<'a>),
}
