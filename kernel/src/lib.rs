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
#![feature(binary_heap_retain)]

extern crate alloc;

mod arch;
mod builder;
mod defmt_log;
mod regions;
mod registry;
mod scheduler;
mod space;
mod syscalls;
mod task;
mod task_ptr;
mod tcb;

use defmt::Format;
use registry::Registry;
use syscalls::{
    CallReturn, CallSysCall, CapsCall, ConnectCall, ListenCall, LogCall, PanikCall, RecvCall,
    SendCall, SysCall,
};
use tcb::*;

pub use builder::*;
#[cfg(test)]
mod tests;

use abi::{Cap, CapRef, Endpoint, SyscallArgs, SyscallIndex, ThreadRef};
use alloc::{boxed::Box, collections::BinaryHeap};
use cordyceps::{
    list::{self, Links},
    List,
};
use core::{ops::Range, pin::Pin};
use heapless::Vec;
use regions::{Region, RegionAttr, RegionTable};
use scheduler::{Scheduler, ThreadTime};
use space::Space;
use task::*;
use task_ptr::{TaskPtr, TaskPtrMut};

pub(crate) const TCB_CAPACITY: usize = 16;

pub struct Kernel {
    pub(crate) scheduler: Scheduler,
    pub(crate) registry: Registry,
    epoch: usize,
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
                    desc.stack_space.clone(),
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
            loaned_tcb: None,
        };
        Ok(Kernel {
            scheduler: Scheduler {
                tcbs: Space::default(),
                current_thread,
                exhausted_threads: List::new(),
                wait_queue: BinaryHeap::default(),
            },
            registry: Registry::default(),
            epoch: 0,
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
        caps: List<CapEntry>,
    ) -> Result<ThreadRef, KernelError> {
        let epoch = self.epoch;
        let task = self.task_mut(task_ref)?;
        let entrypoint = task
            .validate_ptr(entrypoint)
            .ok_or(KernelError::InvalidEntrypoint)?;
        let entrypoint_addr = (entrypoint as *const fn() -> !).addr();
        if task.state != TaskState::Started {
            arch::clear_mem(task);
        }
        let stack = task.alloc_stack().ok_or(KernelError::StackExhausted)?;
        let mut tcb = Tcb::new(
            task_ref,
            stack,
            priority,
            budget,
            cooldown,
            entrypoint_addr,
            epoch,
            caps,
        );
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
        let is_call = reply_endpoint.is_some();
        dest_tcb.req_queue.push_back(Box::pin(IPCMsg {
            reply_endpoint,
            body: IPCMsgBody::Buf(body),
            _links: Links::default(),
            addr: endpoint.addr,
        }));

        if let ThreadState::Waiting { ref recv_req } = dest_tcb.state {
            let addr = recv_req.mask;
            if addr & endpoint.addr == endpoint.addr {
                let recv_req = if let ThreadState::Waiting { recv_req } =
                    core::mem::replace(&mut dest_tcb.state, ThreadState::Ready)
                {
                    recv_req
                } else {
                    unreachable!()
                };
                let task = self
                    .tasks
                    .get_mut(dest_tcb.task.0)
                    .ok_or(KernelError::InvalidTaskRef)?;
                if let RecvRes::NotFound(_) = dest_tcb.recv(task, recv_req)? {
                    panic!("recv not found")
                }
                let dest_tcb_priority = dest_tcb.priority;
                self.scheduler
                    .add_thread(dest_tcb_priority, endpoint.tcb_ref)?;
            }
        } else if is_call {
            let dest_tcb_priority = dest_tcb.priority;
            self.scheduler
                .add_thread(dest_tcb_priority, endpoint.tcb_ref)?;
        }
        Ok(())
    }

    /// Sends a message to an endpoint, and pauses the current thread's execution till a response is
    /// received
    pub(crate) fn call(
        &mut self,
        dest: CapRef,
        msg: Box<[u8]>,
        mut recv_req: RecvReq<'static>,
    ) -> Result<ThreadRef, KernelError> {
        let src_ref = self.scheduler.current_thread.tcb_ref;
        let endpoint = self.scheduler.current_thread_mut()?.endpoint(dest)?;
        let reply_endpoint = Endpoint {
            tcb_ref: src_ref,
            addr: endpoint.addr | 0x80000000,
            disposable: true,
        };
        recv_req.mask = reply_endpoint.addr;
        self.send_inner(endpoint, msg, Some(reply_endpoint))?;
        self.scheduler.wait(recv_req, true) // last bit is flipped for reply TODO(sphw): replace with proper bitmask
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
    ) -> Result<CallReturn, KernelError> {
        let f = index.get(abi::SyscallIndex::SYSCALL_FN);
        if f != abi::SyscallFn::Log {
            defmt::trace!("syscall index: {:?}", f);
        }
        match f {
            abi::SyscallFn::Send => {
                SendCall::from_args(args).exec(index.get(SyscallIndex::SYSCALL_ARG_TYPE), self)
            }
            abi::SyscallFn::Call => {
                CallSysCall::from_args(args).exec(index.get(SyscallIndex::SYSCALL_ARG_TYPE), self)
            }
            abi::SyscallFn::Recv => {
                RecvCall::from_args(args).exec(index.get(SyscallIndex::SYSCALL_ARG_TYPE), self)
            }
            abi::SyscallFn::Log => {
                LogCall::from_args(args).exec(index.get(SyscallIndex::SYSCALL_ARG_TYPE), self)
            }
            abi::SyscallFn::Caps => {
                CapsCall::from_args(args).exec(index.get(SyscallIndex::SYSCALL_ARG_TYPE), self)
            }
            abi::SyscallFn::Panik => {
                PanikCall::from_args(args).exec(index.get(SyscallIndex::SYSCALL_ARG_TYPE), self)
            }
            abi::SyscallFn::Connect => {
                ConnectCall::from_args(args).exec(index.get(SyscallIndex::SYSCALL_ARG_TYPE), self)
            }
            abi::SyscallFn::Listen => {
                ListenCall::from_args(args).exec(index.get(SyscallIndex::SYSCALL_ARG_TYPE), self)
            }
        }
    }
}

#[derive(Debug)]
pub enum KernelError {
    InvalidPriority,
    InvalidThreadRef,
    InvalidTaskRef,
    InvalidEntrypoint,
    TooManyThreads,
    InvalidCapRef,
    StackExhausted,
    InvalidTaskPtr,
    InitTCBNotFound,
    ABI(abi::Error),
}

impl From<abi::Error> for KernelError {
    fn from(v: abi::Error) -> Self {
        Self::ABI(v)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TaskRef(pub usize);

#[derive(PartialEq, Eq, Debug, Format)]
pub(crate) struct DomainEntry {
    tcb_ref: ThreadRef,
    loaned_tcb: Option<ThreadRef>,
    priority: u8,
}

impl PartialOrd for DomainEntry {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.priority.partial_cmp(&other.priority)
    }
}

impl Ord for DomainEntry {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.priority.cmp(&other.priority)
    }
}

impl DomainEntry {
    pub fn new(tcb_ref: ThreadRef, loaned_tcb: Option<ThreadRef>, priority: u8) -> Self {
        Self {
            tcb_ref,
            loaned_tcb,
            priority,
        }
    }

    #[inline]
    pub fn idle() -> Self {
        Self {
            tcb_ref: ThreadRef::idle(),
            loaned_tcb: None,
            priority: 0,
        }
    }
}

#[repr(C)]
enum ThreadState {
    Waiting {
        recv_req: RecvReq<'static>,
    },
    Ready,
    #[allow(dead_code)]
    Running,
}

struct CapEntry {
    _links: list::Links<CapEntry>,
    cap: Cap,
}

pub(crate) struct IPCMsg {
    _links: list::Links<IPCMsg>,
    addr: usize,
    reply_endpoint: Option<Endpoint>,
    body: IPCMsgBody,
}

enum IPCMsgBody {
    Buf(Box<[u8]>),
    Page(TaskPtrMut<'static, [u8]>),
}

#[macro_export]
macro_rules! linked_impl {
    ($t: ty) => {
        // Safety: there a few safety guarantees outlined in [`cordyceps::Linked`], we uphold all of those
        unsafe impl cordyceps::Linked<cordyceps::list::Links<$t>> for $t {
            type Handle = Pin<Box<Self>>;

            fn into_ptr(r: Self::Handle) -> core::ptr::NonNull<Self> {
                // Safety: this is safe, as long as only cordyceps uses `into_ptr`
                unsafe { core::ptr::NonNull::from(Box::leak(Pin::into_inner_unchecked(r))) }
            }

            unsafe fn from_ptr(ptr: core::ptr::NonNull<Self>) -> Self::Handle {
                Pin::new_unchecked(Box::from_raw(ptr.as_ptr()))
            }

            unsafe fn links(
                target: core::ptr::NonNull<Self>,
            ) -> core::ptr::NonNull<cordyceps::list::Links<Self>> {
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

impl TaskDesc {
    fn region_table(&self) -> RegionTable {
        RegionTable {
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
        }
    }
}
