#![cfg_attr(not(any(test, feature = "std")), no_std)]
#![warn(clippy::undocumented_unsafe_blocks)]
#![cfg_attr(test, allow(dead_code))]
#![feature(asm_const)]
#![feature(asm_sym)]
#![feature(ptr_metadata)]
#![feature(strict_provenance)]
#![feature(naked_functions)]
#![feature(maybe_uninit_uninit_array)]
#![feature(maybe_uninit_array_assume_init)]

extern crate alloc;

mod arch;
mod builder;
mod defmt_log;
mod regions;
mod space;
mod task_ptr;

pub use builder::*;
#[cfg(test)]
mod tests;

use abi::{
    Cap, CapListEntry, CapRef, Endpoint, RecvResp, SyscallArgs, SyscallDataType, SyscallIndex,
    SyscallReturn, SyscallReturnType, ThreadRef,
};
use alloc::boxed::Box;
use cordyceps::{
    list::{self, Links},
    List,
};
use core::{
    mem::{self, MaybeUninit},
    ops::Range,
    pin::Pin,
    ptr::NonNull,
};
use heapless::Vec;
use regions::{Region, RegionAttr, RegionTable};
use task_ptr::{TaskPtr, TaskPtrMut};

const PRIORITY_COUNT: usize = 8;

pub struct Kernel {
    pub(crate) scheduler: Scheduler,
    tasks: Vec<Task, 5>,
}

impl Kernel {
    pub fn from_tasks(tasks: &[TaskDesc]) -> Result<Self, KernelError> {
        let tasks: heapless::Vec<_, 5> = tasks
            .iter()
            .map(|desc| {
                Task::new(
                    desc.region_table(),
                    desc.init_stack_size,
                    Vec::from_slice(&[desc.stack_space.clone()]).unwrap(),
                    // Safety: entrypoints are static in k5 currently, so this is safe
                    unsafe { TaskPtr::from_raw_parts(desc.entrypoint, ()) },
                    false,
                )
            })
            .collect();
        let kernel = Kernel::new(tasks)?;
        Ok(kernel)
    }

    pub(crate) fn new(tasks: Vec<Task, 5>) -> Result<Self, KernelError> {
        let current_thread = ThreadTime {
            tcb_ref: ThreadRef(0),
            time: 20,
        };
        let tcbs = Vec::default();
        const DOMAIN_ENTRY: MaybeUninit<List<DomainEntry>> = MaybeUninit::uninit();
        let mut domains: [MaybeUninit<List<DomainEntry>>; PRIORITY_COUNT] =
            [DOMAIN_ENTRY; PRIORITY_COUNT];
        for d in &mut domains {
            d.write(List::new());
        }
        // Safety: We just initialized every item in the array, so this transmute is safe
        let domains: [List<DomainEntry>; PRIORITY_COUNT] = unsafe { mem::transmute(domains) };
        Ok(Kernel {
            scheduler: Scheduler {
                tcbs,
                current_thread,
                exhausted_threads: List::new(),
                domains,
            },
            tasks,
        })
    }

    pub(crate) fn spawn_thread(
        &mut self,
        task_ref: TaskRef,
        priority: usize,
        budget: usize,
        cooldown: usize,
        entrypoint: TaskPtr<'static, fn() -> !>,
    ) -> Result<ThreadRef, KernelError> {
        let task = self.task_mut(task_ref)?;
        let entrypoint = task
            .validate_ptr(entrypoint)
            .ok_or(KernelError::InvalidEntrypoint)?;
        let entrypoint_addr = (entrypoint as *const fn() -> !).addr();
        if task.state != TaskState::Started {
            arch::clear_mem(&task);
        }
        let stack = task.alloc_stack().ok_or(KernelError::StackExhausted)?;
        let mut tcb = TCB::new(task_ref, stack, priority, budget, cooldown, entrypoint_addr);
        arch::init_tcb_stack(task, &mut tcb);
        self.scheduler.spawn(tcb)
    }

    pub(crate) fn task(&self, task_ref: TaskRef) -> Result<&Task, KernelError> {
        self.tasks
            .get(task_ref.0)
            .ok_or(KernelError::InvalidTaskRef)
    }

    pub(crate) fn task_mut(&mut self, task_ref: TaskRef) -> Result<&mut Task, KernelError> {
        self.tasks
            .get_mut(task_ref.0)
            .ok_or(KernelError::InvalidTaskRef)
    }

    /// Sends a message from the current thread to the specified endpoint
    /// This function takes a [`CapRef`] and expects it to be an [`Endpoint`]
    pub(crate) fn send(&mut self, dest: CapRef, msg: Box<[u8]>) -> Result<(), KernelError> {
        let endpoint = self.scheduler.current_thread_mut()?.endpoint(dest)?;
        self.send_inner(endpoint, msg, None)
    }

    fn send_inner(
        &mut self,
        endpoint: Endpoint,
        body: Box<[u8]>,
        reply_endpoint: Option<Endpoint>,
    ) -> Result<(), KernelError> {
        let dest_tcb = self.scheduler.get_tcb_mut(endpoint.tcb_ref)?;
        dest_tcb.req_queue.push_back(Box::pin(IPCMsg {
            inner: Some(IPCMsgInner {
                reply_endpoint,
                body,
            }),
            _links: Links::default(),
            addr: endpoint.addr,
        }));

        if let ThreadState::Waiting { addr, .. } = dest_tcb.state {
            if addr & endpoint.addr == endpoint.addr {
                let (buf, recv_resp) = if let ThreadState::Waiting {
                    out_buf, recv_resp, ..
                } =
                    core::mem::replace(&mut dest_tcb.state, ThreadState::Ready)
                {
                    (out_buf, recv_resp)
                } else {
                    unreachable!()
                };
                let task = self
                    .tasks
                    .get(dest_tcb.task.0)
                    .ok_or(KernelError::InvalidTaskRef)?;
                assert!(dest_tcb.recv(task, addr, buf, recv_resp)?);
                let dest_tcb_priority = dest_tcb.priority;
                self.scheduler
                    .add_thread(dest_tcb_priority, endpoint.tcb_ref)?;
            }
        }
        Ok(())
    }

    /// Sends a message to an endpoint, and pauses the current thread's execution till a response is
    /// received
    pub(crate) fn call(
        &mut self,
        dest: CapRef,
        msg: Box<[u8]>,
        out_buf: TaskPtrMut<'static, [u8]>,
        recv_resp: TaskPtrMut<'static, MaybeUninit<RecvResp>>,
    ) -> Result<ThreadRef, KernelError> {
        let src_ref = self.scheduler.current_thread.tcb_ref;
        let endpoint = self.scheduler.current_thread_mut()?.endpoint(dest)?;
        let reply_endpoint = Endpoint {
            tcb_ref: src_ref,
            addr: endpoint.addr | 0x80000000,
            disposable: true,
        };
        self.send_inner(endpoint, msg, Some(reply_endpoint))?;
        self.wait(endpoint.addr | 0x80000000, out_buf, recv_resp) // last bit is flipped for reply TODO(sphw): replace with proper bitmask
    }

    pub(crate) fn wait(
        &mut self,
        mask: usize,
        out_buf: TaskPtrMut<'static, [u8]>,
        recv_resp: TaskPtrMut<'static, MaybeUninit<RecvResp>>,
    ) -> Result<ThreadRef, KernelError> {
        let src = self.scheduler.current_thread_mut()?;
        src.state = ThreadState::Waiting {
            addr: mask,
            out_buf,
            recv_resp,
        };

        self.scheduler.wait_current_thread()
    }

    pub(crate) fn start(&mut self) -> ! {
        let tcb_ref = self
            .scheduler
            .tick()
            .unwrap()
            .unwrap_or(self.scheduler.current_thread.tcb_ref);
        let tcb = self.scheduler.get_tcb(tcb_ref).unwrap();
        let task = self.task(tcb.task).unwrap();
        arch::start_root_task(task, tcb);
    }

    pub(crate) fn syscall(
        &mut self,
        index: abi::SyscallIndex,
        args: &SyscallArgs,
    ) -> Result<(Option<ThreadRef>, SyscallReturn), KernelError> {
        match index.get(abi::SyscallIndex::SYSCALL_FN) {
            abi::SyscallFn::Send => {
                if index.get(SyscallIndex::SYSCALL_ARG_TYPE) == SyscallDataType::Page {
                    todo!()
                }
                let tcb = self.scheduler.current_thread()?;
                let slice = match self.get_syscall_buf::<1024>(tcb, args) {
                    Ok(s) => s,
                    Err(KernelError::ABI(e)) => {
                        return Ok((None, e.into()));
                    }
                    Err(e) => return Err(e),
                };
                let msg = Box::from(slice);
                let cap = CapRef(args.arg3);
                let priority = tcb.priority;
                self.send(cap, msg)?;
                if let Some(thread) = self.scheduler.next_thread(priority) {
                    Ok((
                        self.scheduler.switch_thread(thread).map(Some)?,
                        SyscallReturn::new()
                            .with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
                    ))
                } else {
                    Ok((
                        None,
                        SyscallReturn::new()
                            .with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
                    ))
                }
            }
            abi::SyscallFn::Call => {
                if index.get(SyscallIndex::SYSCALL_ARG_TYPE) == SyscallDataType::Page {
                    todo!()
                }
                let tcb = self.scheduler.current_thread()?;
                let slice = match self.get_syscall_buf::<1024>(tcb, args) {
                    Ok(s) => s,
                    Err(KernelError::ABI(e)) => {
                        return Ok((None, e.into()));
                    }
                    Err(e) => return Err(e),
                };
                let msg = Box::from(slice);
                let cap = CapRef(args.arg3);
                // Safety: the caller is giving over memory to us, to overwrite
                // TaskPtrMut ensures that the memory belongs to the correct task
                let out_buf = unsafe {
                    TaskPtrMut::<'_, [u8]>::from_raw_parts(args.arg5, args.arg6 as usize)
                };
                // Safety: the caller is giving over memory to us, to overwrite
                // TaskPtrMut ensures that the memory belongs to the correct task
                let recv_resp = unsafe {
                    TaskPtrMut::<'_, MaybeUninit<RecvResp>>::from_raw_parts(args.arg4, ())
                };
                let thread = self.call(cap, msg, out_buf, recv_resp)?;
                Ok((
                    self.scheduler.switch_thread(thread).map(Some)?,
                    SyscallReturn::new(),
                ))
            }
            abi::SyscallFn::Recv => {
                if index.get(SyscallIndex::SYSCALL_ARG_TYPE) == SyscallDataType::Page {
                    todo!()
                }

                // Safety: the caller is giving over memory to us, to overwrite
                // TaskPtrMut ensures that the memory belongs to the correct task
                let out_buf = unsafe {
                    TaskPtrMut::<'_, [u8]>::from_raw_parts(args.arg1, args.arg2 as usize)
                };
                // Safety: the caller is giving over memory to us, to overwrite
                // TaskPtrMut ensures that the memory belongs to the correct task
                let recv_resp = unsafe {
                    TaskPtrMut::<'_, MaybeUninit<RecvResp>>::from_raw_parts(args.arg4, ())
                };

                let tcb = self.scheduler.current_thread_mut()?;
                let task = self
                    .tasks
                    .get(tcb.task.0)
                    .ok_or(KernelError::InvalidTaskRef)?;
                let mask = args.arg3;
                if !tcb.recv(task, mask, out_buf, recv_resp)? {
                    // Safety: the caller is giving over memory to us, to overwrite
                    // TaskPtrMut ensures that the memory belongs to the correct task
                    let out_buf = unsafe {
                        TaskPtrMut::<'_, [u8]>::from_raw_parts(args.arg1, args.arg2 as usize)
                    };
                    // Safety: the caller is giving over memory to us, to overwrite
                    // TaskPtrMut ensures that the memory belongs to the correct task
                    let recv_resp = unsafe {
                        TaskPtrMut::<'_, MaybeUninit<RecvResp>>::from_raw_parts(args.arg4, ())
                    };

                    Ok((
                        Some(self.wait(mask as usize, out_buf, recv_resp)?),
                        SyscallReturn::new(),
                    ))
                } else {
                    Ok((None, SyscallReturn::new()))
                }
            }
            abi::SyscallFn::Log => {
                let tcb = self.scheduler.current_thread()?;
                let log_buf = match self.get_syscall_buf::<255>(tcb, args) {
                    Ok(s) => s,
                    Err(KernelError::ABI(e)) => {
                        return Ok((None, e.into()));
                    }
                    Err(e) => return Err(e),
                };
                crate::defmt_log::log(tcb.task.0 as u8 + 1, log_buf);
                Ok((None, SyscallReturn::new()))
            }
            abi::SyscallFn::Caps => {
                let tcb = self.scheduler.current_thread()?;
                let task = self.task(tcb.task)?;
                // Safety: the caller is giving over memory to us, to overwrite
                // TaskPtrMut ensures that the memory belongs to the correct task
                let slice = unsafe {
                    TaskPtrMut::<'_, [CapListEntry]>::from_raw_parts(args.arg1, args.arg2)
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
                let ret = SyscallReturn::new()
                    .with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy)
                    .with(SyscallReturn::SYSCALL_LEN, len as u64);
                Ok((None, ret))
            }
        }
    }

    #[inline(always)]
    fn get_syscall_buf<const N: usize>(
        &self,
        tcb: &TCB,
        args: &SyscallArgs,
    ) -> Result<&[u8], KernelError> {
        let task = self
            .tasks
            .get(tcb.task.0)
            .ok_or(KernelError::InvalidTaskRef)?;
        // Safety: the caller is giving over memory to us, to overwrite
        // TaskPtrMut ensures that the memory belongs to the correct task
        let slice = unsafe { TaskPtr::<'_, [u8]>::from_raw_parts(args.arg1, args.arg2 as usize) };
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
}

pub(crate) struct Scheduler {
    tcbs: Vec<TCB, 15>,
    domains: [List<DomainEntry>; PRIORITY_COUNT],
    exhausted_threads: List<ExhaustedThread>,
    current_thread: ThreadTime,
}

impl Scheduler {
    pub fn spawn(&mut self, tcb: TCB) -> Result<ThreadRef, KernelError> {
        let d = self
            .domains
            .get_mut(tcb.priority)
            .ok_or(KernelError::InvalidPriority)?;
        let tcb_ref = ThreadRef(self.tcbs.len());
        self.tcbs
            .push(tcb)
            .map_err(|_| KernelError::TooManyThreads)?;
        d.push_back(Box::pin(DomainEntry {
            tcb_ref: Some(tcb_ref),
            ..Default::default()
        }));
        Ok(tcb_ref)
    }
    pub fn next_thread(&mut self, current_priority: usize) -> Option<ThreadRef> {
        for domain in self
            .domains
            .iter_mut()
            .rev()
            .take(PRIORITY_COUNT - 1 - current_priority)
        {
            if let Some(thread) = domain.pop_front().and_then(|t| t.tcb_ref) {
                return Some(thread);
            }
        }
        None
    }

    pub fn add_thread(&mut self, priority: usize, tcb_ref: ThreadRef) -> Result<(), KernelError> {
        let d = self
            .domains
            .get_mut(priority)
            .ok_or(KernelError::InvalidPriority)?;
        d.push_back(Box::pin(DomainEntry::new(tcb_ref)));
        Ok(())
    }

    pub fn tick(&mut self) -> Result<Option<ThreadRef>, KernelError> {
        // requeue exhausted threads
        {
            let mut cursor = self.exhausted_threads.cursor_front_mut();
            cursor.move_prev(); // THIS IS PROBABLY WRONG
            let mut remove_flag = false;
            while let Some(t) = {
                if remove_flag {
                    cursor.remove_current();
                }
                cursor.move_next();
                cursor.current_mut()
            } {
                if let Some(tcb_ref) = t.tcb_ref {
                    if t.decrement() == 0 {
                        remove_flag = true;
                        let tcb = self
                            .tcbs
                            .get(*tcb_ref)
                            .ok_or(KernelError::InvalidThreadRef)?;
                        let d = self
                            .domains
                            .get_mut(tcb.priority)
                            .ok_or(KernelError::InvalidPriority)?;
                        d.push_back(Box::pin(DomainEntry::new(tcb_ref)));
                    }
                }
            }
        }
        self.current_thread.time -= 1;
        // check if current thread's budget has been surpassed
        if self.current_thread.time == 0 {
            let current_tcb = self
                .tcbs
                .get(*self.current_thread.tcb_ref)
                .ok_or(KernelError::InvalidThreadRef)?;
            let exhausted_thread = ExhaustedThread {
                tcb_ref: Some(self.current_thread.tcb_ref),
                time: current_tcb.cooldown,
                _links: Default::default(),
            };
            self.exhausted_threads
                .push_front(Box::pin(exhausted_thread));
            let next_thread = self.next_thread(0).unwrap_or_else(ThreadRef::idle);
            return self.switch_thread(next_thread).map(Some);
        }
        let current_tcb = self.current_thread()?;
        let current_priority = current_tcb.priority;
        if let Some(next_thread) = self.next_thread(current_priority) {
            return self.switch_thread(next_thread).map(Some);
        }
        Ok(None)
    }

    fn switch_thread(&mut self, next_thread: ThreadRef) -> Result<ThreadRef, KernelError> {
        let next_tcb = self
            .tcbs
            .get(*next_thread)
            .ok_or(KernelError::InvalidThreadRef)?;
        self.current_thread = ThreadTime {
            tcb_ref: next_thread,
            time: next_tcb.budget,
        };
        Ok(next_thread)
    }

    fn wait_current_thread(&mut self) -> Result<ThreadRef, KernelError> {
        let next_thread = self.next_thread(0).unwrap_or_else(ThreadRef::idle);
        self.switch_thread(next_thread)
    }

    #[inline]
    pub fn current_thread(&self) -> Result<&TCB, KernelError> {
        self.get_tcb(self.current_thread.tcb_ref)
    }

    #[inline]
    pub fn current_thread_mut(&mut self) -> Result<&mut TCB, KernelError> {
        self.get_tcb_mut(self.current_thread.tcb_ref)
    }

    #[inline]
    pub fn get_tcb(&self, tcb_ref: ThreadRef) -> Result<&TCB, KernelError> {
        self.tcbs.get(*tcb_ref).ok_or(KernelError::InvalidThreadRef)
    }

    #[inline]
    pub fn get_tcb_mut(&mut self, tcb_ref: ThreadRef) -> Result<&mut TCB, KernelError> {
        self.tcbs
            .get_mut(*tcb_ref)
            .ok_or(KernelError::InvalidThreadRef)
    }
}

#[derive(Default)]
struct ExhaustedThread {
    _links: Links<ExhaustedThread>,
    time: usize,
    tcb_ref: Option<ThreadRef>,
}

impl ExhaustedThread {
    fn decrement(self: Pin<&mut ExhaustedThread>) -> usize {
        // Safety: We never move the underlying memory, so this is safe
        unsafe {
            let s = self.get_unchecked_mut();
            s.time -= 1;
            s.time
        }
    }
}

#[derive(Debug)]
struct ThreadTime {
    tcb_ref: ThreadRef,
    time: usize,
}

#[derive(Debug)]
pub enum KernelError {
    InvalidPriority,
    InvalidThreadRef,
    InvalidTaskRef,
    InvalidEntrypoint,
    TooManyThreads,
    InvalidCapRef,
    WrongCapabilityType,
    StackExhausted,
    InvalidTaskPtr,
    ABI(abi::Error),
}

#[derive(Clone, Copy)]
pub struct TaskRef(pub usize);

#[repr(C)]
pub(crate) struct TCB {
    saved_state: arch::SavedThreadState,
    task: TaskRef, // Maybe use RC for this
    req_queue: List<IPCMsg>,
    state: ThreadState,
    priority: usize,
    budget: usize,
    cooldown: usize,
    capabilities: List<CapEntry>,
    stack_pointer: usize,
    entrypoint: usize,
}

impl TCB {
    pub fn new(
        task: TaskRef,
        stack_pointer: usize,
        priority: usize,
        budget: usize,
        cooldown: usize,
        entrypoint: usize,
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
            capabilities: List::new(),
            stack_pointer,
            entrypoint,
            saved_state: Default::default(),
        }
    }

    #[inline]
    fn cap_entry(&self, cap_ref: CapRef) -> Result<&CapEntry, KernelError> {
        for c in self.capabilities.iter() {
            let c_addr = (c as *const CapEntry).addr();
            if c_addr == *cap_ref {
                return Ok(&c);
            }
        }
        Err(KernelError::InvalidCapRef)
    }

    #[allow(dead_code)]
    fn capability(&self, cap_ref: CapRef) -> Result<&Cap, KernelError> {
        self.cap_entry(cap_ref).map(|e| &e.cap)
    }

    fn endpoint(&mut self, cap_ref: CapRef) -> Result<Endpoint, KernelError> {
        let dest_cap = self.cap_entry(cap_ref)?;
        let endpoint = if let Cap::Endpoint(endpoint) = dest_cap.cap {
            endpoint
        } else {
            return Err(KernelError::WrongCapabilityType);
        };
        if endpoint.disposable {
            unsafe {
                self.capabilities.remove(dest_cap.into());
            }
        }
        Ok(endpoint)
    }

    pub fn add_cap(&mut self, cap: Cap) {
        self.capabilities.push_back(Box::pin(CapEntry {
            _links: Links::default(),
            cap,
        }));
    }

    fn recv(
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

#[derive(Default)]
pub(crate) struct DomainEntry {
    _links: list::Links<DomainEntry>,
    tcb_ref: Option<ThreadRef>,
}

impl DomainEntry {
    pub fn new(tcb_ref: ThreadRef) -> Self {
        Self {
            tcb_ref: Some(tcb_ref),
            _links: Default::default(),
        }
    }
}

#[repr(C)]
#[derive(Debug)]
enum ThreadState {
    Waiting {
        addr: usize,
        out_buf: TaskPtrMut<'static, [u8]>,
        recv_resp: TaskPtrMut<'static, MaybeUninit<RecvResp>>,
    },
    Ready,
    #[allow(dead_code)]
    Running,
}

#[repr(C)]
#[derive(Clone)]
pub(crate) struct Task {
    region_table: RegionTable,
    stack_size: usize,
    available_stack_ptr: Vec<Range<usize>, 8>,
    pub entrypoint: TaskPtr<'static, fn() -> !>,
    secure: bool,
    state: TaskState,
}

#[repr(u8)]
#[derive(Clone, PartialEq)]
enum TaskState {
    Pending,
    Started,
    Crashed,
}

impl Task {
    pub fn new(
        region_table: RegionTable,
        stack_size: usize,
        available_stack_ptr: Vec<Range<usize>, 8>,
        entrypoint: TaskPtr<'static, fn() -> !>,
        secure: bool,
    ) -> Self {
        Self {
            region_table,
            stack_size,
            available_stack_ptr,
            secure,
            entrypoint,
            state: TaskState::Pending,
        }
    }

    fn validate_ptr<'a, T: core::ptr::Pointee + ?Sized>(
        &self,
        ptr: TaskPtr<'a, T>,
    ) -> Option<&'a T> {
        arch::translate_task_ptr(ptr, self)
    }

    fn validate_mut_ptr<'a, T: core::ptr::Pointee + ?Sized>(
        &self,
        ptr: TaskPtrMut<'a, T>,
    ) -> Option<&'a mut T> {
        arch::translate_mut_task_ptr(ptr, self)
    }

    pub(crate) fn alloc_stack(&mut self) -> Option<usize> {
        for range in &mut self.available_stack_ptr {
            if range.len() >= self.stack_size {
                range.start += self.stack_size;
                return Some(range.start);
                //TODO: cleanup empty ranges might need to use LL
            }
        }
        None
    }

    #[allow(dead_code)]
    pub(crate) fn make_stack_available(&mut self, stack_start: usize) {
        for range in &mut self.available_stack_ptr {
            if range.start == stack_start + self.stack_size {
                range.start = stack_start;
                return;
            }
            if range.end == stack_start {
                range.end = stack_start + self.stack_size;
                return;
            }
        }
        let _ = self
            .available_stack_ptr
            .push(stack_start..stack_start + self.stack_size);
    }
}

struct CapEntry {
    _links: list::Links<CapEntry>,
    cap: Cap,
}

#[derive(Default)]
pub(crate) struct IPCMsg {
    _links: list::Links<IPCMsg>,
    addr: usize,
    inner: Option<IPCMsgInner>,
}

#[repr(C)]
pub(crate) struct IPCMsgInner {
    reply_endpoint: Option<Endpoint>,
    body: Box<[u8]>,
}

macro_rules! linked_impl {
    ($t: ty) => {
        // Safety: there a few safety guarantees outlined in [`cordyceps::Linked`], we uphold all of those
        unsafe impl cordyceps::Linked<list::Links<$t>> for $t {
            type Handle = Pin<Box<Self>>;

            fn into_ptr(r: Self::Handle) -> core::ptr::NonNull<Self> {
                // Safety: this is safe, as long as only cordyceps uses `into_ptr`
                unsafe { NonNull::from(Box::leak(Pin::into_inner_unchecked(r))) }
            }

            unsafe fn from_ptr(ptr: core::ptr::NonNull<Self>) -> Self::Handle {
                Pin::new_unchecked(Box::from_raw(ptr.as_ptr()))
            }

            unsafe fn links(
                target: core::ptr::NonNull<Self>,
            ) -> core::ptr::NonNull<list::Links<Self>> {
                target.cast()
            }
        }
    };
}

linked_impl! {IPCMsg }
linked_impl! { DomainEntry }
linked_impl! { CapEntry }
linked_impl! { ExhaustedThread }

pub struct TaskDesc {
    pub name: &'static str,
    pub entrypoint: usize,
    pub stack_space: Range<usize>,
    pub init_stack_size: usize,
    pub flash_region: Range<usize>,
    pub ram_region: Range<usize>,
}

impl TaskDesc {
    fn region_table(&self) -> RegionTable {
        let table = RegionTable {
            regions: Vec::from_slice(&[
                Region {
                    range: self.flash_region.clone(),
                    attr: RegionAttr::Exec | RegionAttr::Write | RegionAttr::Read,
                },
                Region {
                    range: self.ram_region.clone(),
                    attr: RegionAttr::Exec | RegionAttr::Write | RegionAttr::Read,
                },
            ])
            .unwrap(),
        };
        table
    }
}
