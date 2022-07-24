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
mod task;
mod task_ptr;
mod tcb;

use tcb::*;

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
};
use defmt::error;
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
            .wait(endpoint.addr | 0x80000000, out_buf, recv_resp) // last bit is flipped for reply TODO(sphw): replace with proper bitmask
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
    ) -> Result<(Option<ThreadRef>, Option<SyscallReturn>), KernelError> {
        match index.get(abi::SyscallIndex::SYSCALL_FN) {
            abi::SyscallFn::Send => {
                if index.get(SyscallIndex::SYSCALL_ARG_TYPE) == SyscallDataType::Page {
                    todo!()
                }
                let tcb = self.scheduler.current_thread()?;
                let slice = match self.get_syscall_buf::<1024>(tcb, args) {
                    Ok(s) => s,
                    Err(KernelError::ABI(e)) => {
                        return Ok((None, Some(e.into())));
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
                        Some(
                            SyscallReturn::new()
                                .with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
                        ),
                    ))
                } else {
                    Ok((
                        None,
                        Some(
                            SyscallReturn::new()
                                .with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Copy),
                        ),
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
                        return Ok((None, Some(e.into())));
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
                    Some(SyscallReturn::new()),
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
                        Some(self.scheduler.wait(mask as usize, out_buf, recv_resp)?),
                        Some(SyscallReturn::new()),
                    ))
                } else {
                    Ok((None, Some(SyscallReturn::new())))
                }
            }
            abi::SyscallFn::Log => {
                let tcb = self.scheduler.current_thread()?;
                let log_buf = match self.get_syscall_buf::<255>(tcb, args) {
                    Ok(s) => s,
                    Err(KernelError::ABI(e)) => {
                        return Ok((None, Some(e.into())));
                    }
                    Err(e) => return Err(e),
                };
                crate::defmt_log::log(tcb.task.0 as u8 + 1, log_buf);
                Ok((None, Some(SyscallReturn::new())))
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
                Ok((None, Some(ret)))
            }
            abi::SyscallFn::Panik => {
                let tcb = self.scheduler.current_thread()?;
                let task_ref = tcb.task;
                error!("task {:?} paniked", task_ref.0);
                for domain in &mut self.scheduler.domains {
                    let mut cursor = domain.cursor_front_mut();
                    cursor.move_prev();
                    while let Some(entry) = { cursor.next() } {
                        if entry
                            .tcb_ref
                            .and_then(|t| self.scheduler.tcbs.get(*t))
                            .is_some()
                        {
                            cursor.remove_current();
                        }
                    }
                }
                let mut priority = None;
                let mut budget = None;
                let mut cooldown = None;
                let task = self
                    .tasks
                    .get_mut(task_ref.0)
                    .ok_or(KernelError::InvalidTaskRef)?;
                task.state = TaskState::Pending;
                task.reset_stack_ptr();
                let task = self
                    .tasks
                    .get(task_ref.0)
                    .ok_or(KernelError::InvalidTaskRef)?;
                for i in 0..16 {
                    if let Some(tcb) = self.scheduler.tcbs.get(i) {
                        if tcb.task == task_ref {
                            if tcb.entrypoint == task.entrypoint.addr() {
                                priority = Some(tcb.priority);
                                budget = Some(tcb.budget);
                                cooldown = Some(tcb.cooldown);
                            }
                            self.scheduler.tcbs.remove(i);
                        }
                    }
                }
                let (priority, budget, cooldown) = if let Some(priority) = priority && let Some(budget) = budget && let Some(cooldown) = cooldown {
                    (priority, budget, cooldown)
                }else {
                    return Err(KernelError:: InitTCBNotFound);
                };
                self.spawn_thread(task_ref, priority, budget, cooldown, task.entrypoint)?;
                let next_thread = self
                    .scheduler
                    .next_thread(0)
                    .unwrap_or_else(ThreadRef::idle);
                let next_thread = self.scheduler.switch_thread(next_thread)?;
                Ok((Some(next_thread), None))
            }
        }
    }

    #[inline(always)]
    fn get_syscall_buf<const N: usize>(
        &self,
        tcb: &Tcb,
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
