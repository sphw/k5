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
mod registry;
mod scheduler;
mod space;
mod syscalls;
mod task;
mod task_ptr;
mod tcb;

use registry::Registry;
use syscalls::{
    CallReturn, CallSysCall, CapsCall, ConnectCall, ListenCall, LogCall, PanikCall, RecvCall,
    SendCall, SysCall,
};
use tcb::*;

pub use builder::*;
#[cfg(test)]
mod tests;

use abi::{Cap, CapRef, Endpoint, RecvResp, SyscallArgs, SyscallIndex, ThreadRef};
use alloc::boxed::Box;
use cordyceps::{
    list::{self, Links},
    List,
};
use core::{
    mem::{self, MaybeUninit},
    ops::Range,
    pin::Pin,
};
use heapless::Vec;
use regions::{Region, RegionAttr, RegionTable};
use scheduler::{Scheduler, ThreadTime};
use space::Space;
use task::*;
use task_ptr::{TaskPtr, TaskPtrMut};

pub(crate) const PRIORITY_COUNT: usize = 8;
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
                tcbs: Space::default(),
                current_thread,
                exhausted_threads: List::new(),
                domains,
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
        self.scheduler
            .wait(endpoint.addr | 0x80000000, out_buf, recv_resp, true) // last bit is flipped for reply TODO(sphw): replace with proper bitmask
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
        match index.get(abi::SyscallIndex::SYSCALL_FN) {
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TaskRef(pub usize);

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
