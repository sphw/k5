#![no_std]
#![allow(dead_code)]
extern crate alloc;

pub mod cortexm;
use alloc::boxed::Box;
use cordyceps::{
    list::{self, Links},
    mpsc_queue, List, MpscQueue,
};
use core::{
    marker::PhantomData,
    mem::{self, MaybeUninit},
    ops::{Deref, Range},
    pin::Pin,
    ptr::NonNull,
};
use heapless::Vec;

const PRIORITY_COUNT: usize = 8;

pub struct Kernel<A: Arch> {
    pub scheduler: Scheduler,
    pub arch_phantom: PhantomData<A>,
}

impl<A: Arch> Kernel<A> {
    pub fn new(idle: TCB) -> Self {
        let current_thread = ThreadTime {
            tcb_ref: ThreadRef(0),
            time: idle.budget,
        };

        let mut tcbs = Vec::default();
        tcbs.push(idle).map_err(|_| ()).unwrap(); // this can't error since its the first item
        let domains = MaybeUninit::<[MpscQueue<DomainEntry>; PRIORITY_COUNT]>::uninit();
        let mut domains: [MaybeUninit<MpscQueue<DomainEntry>>; PRIORITY_COUNT] =
            unsafe { mem::transmute(domains) };
        for d in &mut domains {
            d.write(MpscQueue::new_with_stub(Box::pin(DomainEntry::default())));
        }
        let domains: [MpscQueue<DomainEntry>; PRIORITY_COUNT] = unsafe { mem::transmute(domains) };
        Kernel {
            scheduler: Scheduler {
                tcbs,
                current_thread,
                exhausted_threads: List::new(),
                domains,
            },
            arch_phantom: PhantomData::default(),
        }
    }

    /// Sends a message from the current thread to the specified endpoint
    /// This function takes a [`CapabilityRef`] and expects it to be an [`Endpoint`]
    pub fn send<T>(&mut self, dest: CapabilityRef, msg: &T) -> Result<(), KernelError> {
        let endpoint = self.scheduler.current_thread()?.endpoint(dest)?;
        self.send_inner(endpoint, msg, None)
    }

    fn send_inner<T>(
        &mut self,
        endpoint: Endpoint,
        msg: &T,
        reply_thread: Option<ThreadRef>,
    ) -> Result<(), KernelError> {
        let dest_tcb = self.scheduler.get_tcb_mut(endpoint.tcb_ref)?;
        dest_tcb.req_queue.enqueue(Box::pin(IPCMsg {
            inner: Some(IPCMsgInner {
                reply_thread,
                body: unsafe { mem::transmute(msg) },
                len: mem::size_of_val(msg),
            }),
            ..Default::default()
        }));
        if let ThreadState::Waiting(mask) = dest_tcb.state {
            if mask & endpoint.addr == endpoint.addr {
                dest_tcb.state = ThreadState::Ready;
                let dest_tcb_priority = dest_tcb.priority;
                self.scheduler
                    .add_thread(dest_tcb_priority, endpoint.tcb_ref)?;
            }
        }
        Ok(())
    }

    /// Sends a message to an endpoint, and pauses the current thread's execution till a response is
    /// received
    pub fn call<T>(&mut self, dest: CapabilityRef, msg: &T) -> Result<ThreadRef, KernelError> {
        let src_ref = self.scheduler.current_thread.tcb_ref;
        let endpoint = self.scheduler.current_thread()?.endpoint(dest)?;
        self.send_inner(endpoint, msg, Some(src_ref))?;
        self.wait(endpoint.addr | 0x80000000) // last bit is flipped for reply TODO(sphw): replace with proper bitmask
    }

    pub fn wait(&mut self, mask: usize) -> Result<ThreadRef, KernelError> {
        let src = self.scheduler.current_thread_mut()?;
        src.state = ThreadState::Waiting(mask);
        self.scheduler.wait_current_thread()
    }
}

pub struct Scheduler {
    tcbs: Vec<TCB, 64>,
    domains: [MpscQueue<DomainEntry>; PRIORITY_COUNT],
    exhausted_threads: List<ExhaustedThread>,
    current_thread: ThreadTime,
}

impl Scheduler {
    pub fn spawn(&mut self, tcb: TCB) -> Result<(), KernelError> {
        let d = self
            .domains
            .get(tcb.priority)
            .ok_or(KernelError::InvalidPriority)?;
        let tcb_ref = Some(ThreadRef(self.tcbs.len()));
        self.tcbs
            .push(tcb)
            .map_err(|_| KernelError::TooManyThreads)?;
        d.enqueue(Box::pin(DomainEntry {
            tcb_ref,
            ..Default::default()
        }));
        Ok(())
    }
    pub fn next_thread(&mut self, current_priority: usize) -> Option<ThreadRef> {
        for domain in self
            .domains
            .iter_mut()
            .rev()
            .take(PRIORITY_COUNT - 1 - current_priority)
        {
            if let Some(thread) = domain.dequeue().and_then(|t| t.tcb_ref) {
                return Some(thread);
            }
        }
        None
    }

    pub fn add_thread(&self, priority: usize, tcb_ref: ThreadRef) -> Result<(), KernelError> {
        let d = self
            .domains
            .get(priority)
            .ok_or(KernelError::InvalidPriority)?;
        d.enqueue(Box::pin(DomainEntry::new(tcb_ref)));
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
                            .get(tcb.priority)
                            .ok_or(KernelError::InvalidPriority)?;
                        d.enqueue(Box::pin(DomainEntry::new(tcb_ref)));
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
    TooManyThreads,
    InvalidCapabilityRef,
    WrongCapabilityType,
}

pub trait Arch {
    fn init();
    fn context_switch(tcb: &TCB);
}

pub struct TaskRef(usize);

pub struct TCB {
    task: TaskRef, // Maybe use RC for this
    req_queue: MpscQueue<IPCMsg>,
    reply_queue: MpscQueue<IPCMsg>,
    state: ThreadState,
    priority: usize,
    budget: usize,
    cooldown: usize,
    capabilities: Vec<Capability, 32>,
}

impl TCB {
    fn endpoint(&self, cap_ref: CapabilityRef) -> Result<Endpoint, KernelError> {
        let dest_cap = self
            .capabilities
            .get(*cap_ref)
            .ok_or(KernelError::InvalidCapabilityRef)?;
        let endpoint = if let Capability::Endpoint(endpoint) = dest_cap {
            endpoint
        } else {
            return Err(KernelError::WrongCapabilityType);
        };
        Ok(*endpoint)
    }
}

#[derive(Default)]
pub struct DomainEntry {
    links: mpsc_queue::Links<DomainEntry>,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreadRef(usize);

impl Deref for ThreadRef {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ThreadRef {
    const fn idle() -> ThreadRef {
        ThreadRef(0)
    }
}

#[derive(Debug)]
enum ThreadState {
    Waiting(usize),
    Ready,
    Running,
}

impl TCB {
    pub fn new(task: TaskRef, priority: usize, budget: usize, cooldown: usize) -> Self {
        Self {
            task,
            req_queue: MpscQueue::new_with_stub(Box::pin(IPCMsg::default())),
            reply_queue: MpscQueue::new_with_stub(Box::pin(IPCMsg::default())),
            state: ThreadState::Ready,
            priority,
            budget,
            cooldown,
            capabilities: Vec::default(),
        }
    }
}

#[repr(C)]
pub struct Task {
    core_memory_region: Range<usize>,
    capabilities: Vec<Capability, 32>,
    secure: bool,
}

#[derive(Default)]
pub struct IPCMsg {
    links: list::Links<IPCMsg>,
    inner: Option<IPCMsgInner>,
}

#[repr(C)]
pub struct IPCMsgInner {
    reply_thread: Option<ThreadRef>,
    body: *const (),
    len: usize,
}

macro_rules! linked_impl {
    ($t: ty) => {
        unsafe impl cordyceps::Linked<mpsc_queue::Links<$t>> for $t {
            type Handle = Pin<Box<Self>>;

            fn into_ptr(r: Self::Handle) -> core::ptr::NonNull<Self> {
                unsafe { NonNull::from(Box::leak(Pin::into_inner_unchecked(r))) }
            }

            unsafe fn from_ptr(ptr: core::ptr::NonNull<Self>) -> Self::Handle {
                Pin::new_unchecked(Box::from_raw(ptr.as_ptr()))
            }

            unsafe fn links(
                target: core::ptr::NonNull<Self>,
            ) -> core::ptr::NonNull<mpsc_queue::Links<Self>> {
                target.cast()
            }
        }
    };
}

linked_impl! {IPCMsg }
linked_impl! { DomainEntry }

pub enum Capability {
    Endpoint(Endpoint),
    Notification,
}

pub struct CapabilityRef(usize);
impl Deref for CapabilityRef {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Copy)]
pub struct Endpoint {
    tcb_ref: ThreadRef,
    addr: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestArch;

    type Kernel = super::Kernel<TestArch>;

    impl Arch for TestArch {
        fn init() {}

        fn context_switch(_tcb: &TCB) {}
    }

    #[test]
    fn test_simple_tick_schedule() {
        let mut kernel = Kernel::new(TCB::new(TaskRef(0), 0, usize::MAX, 0));
        let a = TCB::new(TaskRef(1), 7, 5, 6);
        let b = TCB::new(TaskRef(2), 7, 3, 3);
        kernel.scheduler.spawn(a).unwrap();
        kernel.scheduler.spawn(b).unwrap();
        for _ in 0..5 {
            let next = kernel
                .scheduler
                .tick()
                .unwrap()
                .expect("should switch to a");
            assert_eq!(*next, 1, "should switch to a");
            for _ in 0..4 {
                let next = kernel.scheduler.tick().unwrap();
                assert_eq!(next, None);
            }
            let next = kernel
                .scheduler
                .tick()
                .unwrap()
                .expect("should switch to b");
            assert_eq!(*next, 2);
            for _ in 0..2 {
                let next = kernel.scheduler.tick().unwrap();
                assert_eq!(next, None);
            }
            let next = kernel
                .scheduler
                .tick()
                .unwrap()
                .expect("should switch to idle");
            assert_eq!(*next, 0);
            for _ in 0..2 {
                let next = kernel.scheduler.tick().unwrap();
                assert_eq!(next, None);
            }
            let next = kernel
                .scheduler
                .tick()
                .unwrap()
                .expect("should switch to b");
            assert_eq!(*next, 2);
            for _ in 0..2 {
                let next = kernel.scheduler.tick().unwrap();
                assert_eq!(next, None);
            }
        }
    }

    #[test]
    fn test_send_schedule() {
        let mut kernel = Kernel::new(TCB::new(TaskRef(0), 0, usize::MAX, 0));
        let a = TCB::new(TaskRef(1), 7, 5, 6);
        let mut b = TCB::new(TaskRef(2), 7, 3, 3);
        b.capabilities
            .push(Capability::Endpoint(Endpoint {
                tcb_ref: ThreadRef(1),
                addr: 1,
            }))
            .map_err(|_| ())
            .unwrap();
        kernel.scheduler.spawn(a).unwrap();
        kernel.scheduler.spawn(b).unwrap();
        let next = kernel
            .scheduler
            .tick()
            .unwrap()
            .expect("should switch to a");
        assert_eq!(*next, 1, "should switch to a");
        let next = kernel.wait(0x1).unwrap();
        assert_eq!(*next, 2, "should switch to b");
        let msg = [1u8, 2, 3];
        kernel.send(CapabilityRef(0), &msg).expect("send failed");
        for _ in 0..2 {
            let next = kernel.scheduler.tick().unwrap();
            assert_eq!(next, None);
        }
        let next = kernel
            .scheduler
            .tick()
            .unwrap()
            .expect("should switch to a");
        assert_eq!(*next, 1, "should switch to a");
    }

    #[test]
    fn test_call_schedule() {
        let mut kernel = Kernel::new(TCB::new(TaskRef(0), 0, usize::MAX, 0));
        let a = TCB::new(TaskRef(1), 7, 5, 6);
        let mut b = TCB::new(TaskRef(2), 7, 3, 3);
        b.capabilities
            .push(Capability::Endpoint(Endpoint {
                tcb_ref: ThreadRef(1),
                addr: 1,
            }))
            .map_err(|_| ())
            .unwrap();
        kernel.scheduler.spawn(a).unwrap();
        kernel.scheduler.spawn(b).unwrap();
        let next = kernel
            .scheduler
            .tick()
            .unwrap()
            .expect("should switch to a");
        assert_eq!(*next, 1, "should switch to a");
        let next = kernel.wait(0x1).unwrap();
        assert_eq!(*next, 2, "should switch to b");
        let msg = [1u8, 2, 3];
        let next = kernel.call(CapabilityRef(0), &msg).expect("send failed");
        assert_eq!(*next, 1, "should switch to a");
    }
}
