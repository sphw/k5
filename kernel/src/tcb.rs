use core::mem::MaybeUninit;

use abi::{Cap, CapRef, Endpoint, RecvResp, SyscallReturn, SyscallReturnType};
use alloc::boxed::Box;
use cordyceps::{list::Links, List};

use crate::{
    arch, task_ptr::TaskPtrMut, CapEntry, IPCMsg, KernelError, Task, TaskRef, ThreadState,
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

    pub(crate) fn recv(
        &mut self,
        task: &Task,
        mask: usize,
        out_buf: TaskPtrMut<'_, [u8]>,
        recv_resp: TaskPtrMut<'_, MaybeUninit<RecvResp>>,
    ) -> Result<bool, KernelError> {
        let mut cursor = self.req_queue.cursor_front_mut();
        cursor.move_prev();
        let mut found = false;
        while let Some(msg) = {
            cursor.move_next();
            cursor.current()
        } {
            if msg.addr & mask == mask {
                found = true;
                break;
            }
        }
        if !found {
            return Ok(false);
        }
        let msg = cursor.remove_current().unwrap(); // found will only ever be set when there is a msg
        let out_buf = task
            .validate_mut_ptr(out_buf)
            .ok_or(KernelError::InvalidTaskPtr)?;
        let inner = &msg.inner.as_ref().unwrap();
        if out_buf.len() != inner.body.len() {
            self.saved_state
                .set_syscall_return(abi::Error::ReturnTypeMismatch.into());
            return Ok(true);
        }
        out_buf.copy_from_slice(&inner.body);
        self.saved_state.set_syscall_return(
            SyscallReturn::new().with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
        );
        let recv_resp = task
            .validate_mut_ptr(recv_resp)
            .ok_or(KernelError::InvalidTaskPtr)?; // TODO: make syscall return
        let recv_resp = recv_resp.write(RecvResp {
            cap: None,
            inner: abi::RecvRespInner::Copy(inner.body.len()),
        });
        if let Some(reply) = inner.reply_endpoint {
            self.add_cap(Cap::Endpoint(reply));
            let cap_ptr = &*self.capabilities.back().unwrap() as *const CapEntry;
            recv_resp.cap = Some(CapRef(cap_ptr.addr()));
        }
        drop(msg);
        Ok(true)
    }
}
