#![cfg_attr(not(any(test, feature = "std")), no_std)]
#![allow(dead_code)]
#![feature(asm_const)]
#![feature(asm_sym)]
#![feature(ptr_metadata)]
#![feature(strict_provenance)]
#![feature(naked_functions)]

extern crate alloc;

pub mod arch;
pub mod task_ptr;

#[cfg(test)]
mod tests;

use abi::{
    CapListEntry, Capability, CapabilityRef, Endpoint, SyscallArgs, SyscallDataType, SyscallIndex,
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
use task_ptr::{TaskPtr, TaskPtrMut};

const PRIORITY_COUNT: usize = 8;

pub struct Kernel {
    pub scheduler: Scheduler,
    tasks: Vec<Task, 5>,
}

impl Kernel {
    pub fn from_tasks(tasks: &[TaskDesc], idle_index: usize) -> Result<Self, KernelError> {
        let tasks: heapless::Vec<_, 5> = tasks
            .iter()
            .map(|desc| {
                Task::new(
                    desc.flash_region.clone(),
                    desc.ram_region.clone(),
                    desc.init_stack_size,
                    Vec::from_slice(&[desc.stack_space.clone()]).unwrap(),
                    Vec::default(),
                    unsafe { TaskPtr::from_raw_parts(desc.entrypoint, ()) },
                    false,
                )
            })
            .collect();
        let kernel = Kernel::new(tasks, TaskRef(idle_index))?;
        Ok(kernel)
    }

    pub fn new(tasks: Vec<Task, 5>, idle_ref: TaskRef) -> Result<Self, KernelError> {
        let current_thread = ThreadTime {
            tcb_ref: ThreadRef(0),
            time: 20,
        };
        let tcbs = Vec::default();
        let domains = MaybeUninit::<[List<DomainEntry>; PRIORITY_COUNT]>::uninit();
        let mut domains: [MaybeUninit<List<DomainEntry>>; PRIORITY_COUNT] =
            unsafe { mem::transmute(domains) };
        for d in &mut domains {
            d.write(List::new());
        }
        let domains: [List<DomainEntry>; PRIORITY_COUNT] = unsafe { mem::transmute(domains) };
        let mut kernel = Kernel {
            scheduler: Scheduler {
                tcbs,
                current_thread,
                exhausted_threads: List::new(),
                domains,
            },
            tasks,
        };
        let task = kernel.task(idle_ref)?;
        kernel.spawn_thread(idle_ref, 0, usize::MAX, 0, task.entrypoint)?;
        Ok(kernel)
    }

    pub fn spawn_thread(
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
        let stack = task.alloc_stack().ok_or(KernelError::StackExhausted)?;
        let mut tcb = TCB::new(task_ref, stack, priority, budget, cooldown, entrypoint_addr);
        arch::init_tcb_stack(task, &mut tcb);
        self.scheduler.spawn(tcb)
    }

    pub fn task(&self, task_ref: TaskRef) -> Result<&Task, KernelError> {
        self.tasks
            .get(task_ref.0)
            .ok_or(KernelError::InvalidTaskRef)
    }

    pub fn task_mut(&mut self, task_ref: TaskRef) -> Result<&mut Task, KernelError> {
        self.tasks
            .get_mut(task_ref.0)
            .ok_or(KernelError::InvalidTaskRef)
    }

    /// Sends a message from the current thread to the specified endpoint
    /// This function takes a [`CapabilityRef`] and expects it to be an [`Endpoint`]
    pub fn send(&mut self, dest: CapabilityRef, msg: Box<[u8]>) -> Result<(), KernelError> {
        let endpoint = self.scheduler.current_thread()?.endpoint(dest)?;
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
            ..Default::default()
        }));

        if let ThreadState::Waiting(mask, _) = dest_tcb.state {
            if mask & endpoint.addr == endpoint.addr {
                let mut buf = if let ThreadState::Waiting(_, buf) =
                    core::mem::replace(&mut dest_tcb.state, ThreadState::Ready)
                {
                    buf
                } else {
                    unreachable!()
                };
                let task = self
                    .tasks
                    .get(dest_tcb.task.0)
                    .ok_or(KernelError::InvalidTaskRef)?;
                assert!(dest_tcb.pop_msg(task, mask, &mut buf)?);
                let dest_tcb_priority = dest_tcb.priority;
                self.scheduler
                    .add_thread(dest_tcb_priority, endpoint.tcb_ref)?;
            }
        }
        Ok(())
    }

    /// Sends a message to an endpoint, and pauses the current thread's execution till a response is
    /// received
    pub fn call(
        &mut self,
        dest: CapabilityRef,
        msg: Box<[u8]>,
        out_buf: TaskPtrMut<'static, [u8]>,
    ) -> Result<ThreadRef, KernelError> {
        let src_ref = self.scheduler.current_thread.tcb_ref;
        let endpoint = self.scheduler.current_thread()?.endpoint(dest)?;
        let reply_endpoint = Endpoint {
            tcb_ref: src_ref,
            addr: endpoint.addr | 0x80000000,
        };
        self.send_inner(endpoint, msg, Some(reply_endpoint))?;
        self.wait(endpoint.addr | 0x80000000, out_buf) // last bit is flipped for reply TODO(sphw): replace with proper bitmask
    }

    pub fn wait(
        &mut self,
        mask: usize,
        out_buf: TaskPtrMut<'static, [u8]>,
    ) -> Result<ThreadRef, KernelError> {
        let src = self.scheduler.current_thread_mut()?;
        src.state = ThreadState::Waiting(mask, out_buf);
        self.scheduler.wait_current_thread()
    }

    pub fn start(&mut self) -> ! {
        let tcb_ref = self
            .scheduler
            .tick()
            .unwrap()
            .unwrap_or(self.scheduler.current_thread.tcb_ref);
        let tcb = self.scheduler.get_tcb(tcb_ref).unwrap();
        arch::start_root_task(tcb);
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
                let cap = CapabilityRef(args.arg3);
                let priority = tcb.priority;
                self.send(cap, msg)?;
                if let Some(thread) = self.scheduler.next_thread(priority) {
                    Ok((
                        self.scheduler.switch_thread(thread).map(Some)?,
                        SyscallReturn::new(),
                    ))
                } else {
                    Ok((None, SyscallReturn::new()))
                }
            }
            abi::SyscallFn::Call => todo!(),
            abi::SyscallFn::Recv => {
                if index.get(SyscallIndex::SYSCALL_ARG_TYPE) == SyscallDataType::Page {
                    todo!()
                }
                let mut out_buf: TaskPtrMut<'_, [u8]> =
                    unsafe { TaskPtrMut::from_raw_parts(args.arg1, args.arg2 as usize) };
                let tcb = self.scheduler.current_thread_mut()?;
                let task = self
                    .tasks
                    .get(tcb.task.0)
                    .ok_or(KernelError::InvalidTaskRef)?;
                let mask = args.arg3;
                if !tcb.pop_msg(task, mask, &mut out_buf)? {
                    Ok((
                        Some(self.wait(mask as usize, out_buf)?),
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
                let mut buf = [0u8; 257];
                buf[0] = tcb.task.0 as u8;
                buf[1] = log_buf.len() as u8;
                // NOTE: this assumes that the internal task index is the same as codegen task index, which is true for embedded,
                // but for systems with dynamic tasks is not true.
                buf[2..log_buf.len() + 2].clone_from_slice(log_buf);
                unsafe { arch::log(&buf[..log_buf.len() + 2]) };
                Ok((None, SyscallReturn::new()))
            }
            abi::SyscallFn::Caps => {
                //TODO(sphw): check length bounds
                //TODO(sphw): refactor into own func
                let tcb = self.scheduler.current_thread()?;
                let slice = unsafe {
                    core::slice::from_raw_parts_mut(
                        args.arg1 as *mut CapListEntry,
                        args.arg2 as usize,
                    )
                };
                let len = slice.len().min(tcb.capabilities.len());
                for (i, entry) in tcb.capabilities.iter().take(len).enumerate() {
                    slice[i] = abi::CapListEntry {
                        cap_ref: CapabilityRef((entry as *const CapEntry).addr()),
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
        let slice: TaskPtr<'_, [u8]> =
            unsafe { TaskPtr::from_raw_parts(args.arg1, args.arg2 as usize) };
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

pub struct Scheduler {
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
                links: Default::default(),
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
    links: Links<ExhaustedThread>,
    time: usize,
    tcb_ref: Option<ThreadRef>,
}

impl ExhaustedThread {
    fn tcb_ref(self: Pin<&mut ExhaustedThread>) -> Option<ThreadRef> {
        self.tcb_ref
    }
    fn decrement(self: Pin<&mut ExhaustedThread>) -> usize {
        // Saftey: We never consider this pinned
        unsafe {
            let s = self.get_unchecked_mut();
            s.time -= 1;
            s.time
        }
    }
}

unsafe impl cordyceps::Linked<list::Links<ExhaustedThread>> for ExhaustedThread {
    type Handle = Pin<Box<Self>>;

    fn into_ptr(r: Self::Handle) -> core::ptr::NonNull<Self> {
        unsafe { NonNull::from(Box::leak(Pin::into_inner_unchecked(r))) }
    }

    unsafe fn from_ptr(ptr: core::ptr::NonNull<Self>) -> Self::Handle {
        Pin::new_unchecked(Box::from_raw(ptr.as_ptr()))
    }

    unsafe fn links(target: core::ptr::NonNull<Self>) -> core::ptr::NonNull<list::Links<Self>> {
        target.cast()
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
    InvalidCapabilityRef,
    WrongCapabilityType,
    StackExhausted,
    InvalidTaskPtr,
    ABI(abi::Error),
}

#[derive(Clone, Copy)]
pub struct TaskRef(pub usize);

#[repr(C)]
pub struct TCB {
    saved_state: arch::SavedThreadState,
    //_pad: usize,
    task: TaskRef, // Maybe use RC for this
    req_queue: List<IPCMsg>,
    //reply_queue: List<IPCMsg>,
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

    fn capability(&self, cap_ref: CapabilityRef) -> Result<&Capability, KernelError> {
        for c in self.capabilities.iter() {
            let c_addr = (c as *const CapEntry).addr();
            if c_addr == *cap_ref {
                return Ok(&c.cap);
            }
        }
        Err(KernelError::InvalidCapabilityRef)
    }

    fn endpoint(&self, cap_ref: CapabilityRef) -> Result<Endpoint, KernelError> {
        let dest_cap = self.capability(cap_ref)?;
        let endpoint = if let Capability::Endpoint(endpoint) = dest_cap {
            endpoint
        } else {
            return Err(KernelError::WrongCapabilityType);
        };
        Ok(*endpoint)
    }

    pub fn add_cap(&mut self, cap: Capability) {
        let _ = self.capabilities.push_back(Box::pin(CapEntry {
            links: Links::default(),
            cap,
        }));
    }

    fn pop_msg(
        &mut self,
        task: &Task,
        mask: usize,
        out_buf: &mut TaskPtrMut<'_, [u8]>,
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
        let body = &msg.inner.as_ref().unwrap().body;
        if out_buf.len() != body.len() {
            self.saved_state
                .set_syscall_return(abi::Error::ReturnTypeMismatch.into());
            return Ok(true);
        }
        out_buf.copy_from_slice(body);
        self.saved_state.set_syscall_return(
            SyscallReturn::new().with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
        );
        Ok(true)
    }
}

#[derive(Default)]
pub struct DomainEntry {
    links: list::Links<DomainEntry>,
    tcb_ref: Option<ThreadRef>,
}

impl DomainEntry {
    pub fn new(tcb_ref: ThreadRef) -> Self {
        Self {
            tcb_ref: Some(tcb_ref),
            links: Default::default(),
        }
    }
}

#[derive(Debug)]
enum ThreadState {
    Waiting(usize, TaskPtrMut<'static, [u8]>),
    Ready,
    Running,
}

#[repr(C)]
#[derive(Clone)]
pub struct Task {
    flash_memory_region: Range<usize>,
    ram_memory_region: Range<usize>,
    stack_size: usize,
    available_stack_ptr: Vec<Range<usize>, 8>,
    capabilities: Vec<Capability, 10>,
    pub entrypoint: TaskPtr<'static, fn() -> !>,
    secure: bool,
}

impl Task {
    pub fn new(
        flash_memory_region: Range<usize>,
        ram_memory_region: Range<usize>,
        stack_size: usize,
        available_stack_ptr: Vec<Range<usize>, 8>,
        capabilities: Vec<Capability, 10>,
        entrypoint: TaskPtr<'static, fn() -> !>,
        secure: bool,
    ) -> Self {
        Self {
            flash_memory_region,
            ram_memory_region,
            stack_size,
            available_stack_ptr,
            capabilities,
            secure,
            entrypoint,
        }
    }

    fn validate_ptr<'a, T: core::ptr::Pointee + ?Sized>(
        &self,
        ptr: TaskPtr<'a, T>,
    ) -> Option<&'a T> {
        unsafe {
            ptr.validate(&self.ram_memory_region)
                .or_else(|| ptr.validate(&self.flash_memory_region))
        }
    }

    fn validate_mut_ptr<'a, 'r, T: core::ptr::Pointee + ?Sized>(
        &self,
        ptr: &'r mut TaskPtrMut<'a, T>,
    ) -> Option<&'r mut T> {
        unsafe { ptr.validate(&self.ram_memory_region, &self.flash_memory_region) }
    }

    fn alloc_stack(&mut self) -> Option<usize> {
        for range in &mut self.available_stack_ptr {
            if range.len() >= self.stack_size {
                range.start += self.stack_size;
                return Some(range.start);
                //TODO: cleanup empty ranges might need to use LL
            }
        }
        None
    }

    fn make_stack_available(&mut self, stack_start: usize) {
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
    links: list::Links<CapEntry>,
    cap: Capability,
}

#[derive(Default)]
pub struct IPCMsg {
    links: list::Links<IPCMsg>,
    addr: usize,
    inner: Option<IPCMsgInner>,
}

#[repr(C)]
pub struct IPCMsgInner {
    reply_endpoint: Option<Endpoint>,
    body: Box<[u8]>,
}

macro_rules! linked_impl {
    ($t: ty) => {
        unsafe impl cordyceps::Linked<list::Links<$t>> for $t {
            type Handle = Pin<Box<Self>>;

            fn into_ptr(r: Self::Handle) -> core::ptr::NonNull<Self> {
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

pub struct TaskDesc {
    pub name: &'static str,
    pub entrypoint: usize,
    pub stack_space: Range<usize>,
    pub init_stack_size: usize,
    pub flash_region: Range<usize>,
    pub ram_region: Range<usize>,
}
