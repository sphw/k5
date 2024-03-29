use core::{mem, ops::Range};

use abi::{Cap, Endpoint, Listen, PortId, ThreadRef};
use alloc::boxed::Box;
use cordyceps::List;
use enumflags2::BitFlags;

use crate::{
    regions::{Region, RegionAttr},
    CapEntry, Kernel, KernelError, TaskDesc, TaskRef,
};

/// Builder for creating and booting the k5 kernel
///
/// Each K5 app should use `KernelBuilder` to initialize tasks, and their capabilities
/// You are required to set an idle thread using [`KernelBuilder::idle_thread`]. The idle threads runs at the lowest
/// priority, has a cooldown of 0 and an infinite budget. This means that if no other thread is scheduable
/// the idle thread will be run.
pub struct KernelBuilder<'a> {
    cycles_per_tick: usize,
    idle_task_set: bool,
    kernel: &'a mut Kernel,
}

impl KernelBuilder<'_> {
    pub fn new(tasks: &[TaskDesc]) -> Self {
        assert!(
            !tasks.is_empty(),
            "must have at least one task to start kernel"
        );
        Self {
            cycles_per_tick: 400_000,
            kernel: crate::arch::init_kernel(tasks),
            idle_task_set: false,
        }
    }

    /// Set's the cycles per kernel tick
    ///
    /// Budget and cooldown are based on this tick, so changing the cyclecount
    /// will change the behavior of your application.
    pub fn cycles_per_tick(&mut self, cycles: usize) -> &mut Self {
        self.cycles_per_tick = cycles;
        self
    }

    /// Spawns a new thread, and retunrs the thread buf
    pub fn thread(&mut self, thread: ThreadBuilder) -> ThreadRef {
        let task_ref = TaskRef(thread.index);
        let task = self
            .kernel
            .task_mut(task_ref)
            .expect("invalid thread index");
        let entrypoint = task.entrypoint;
        for loan in thread.loans.into_iter() {
            task.region_table
                .push(loan.build())
                .expect("loan add failed");
        }
        self.kernel
            .spawn_thread(
                task_ref,
                thread.priority,
                thread.budget,
                thread.cooldown,
                entrypoint,
                thread.caps,
            )
            .unwrap()
    }

    /// Spawns the idle thread, this must be run at least once per builder
    pub fn idle_thread(&mut self, thread: ThreadBuilder) -> ThreadRef {
        let task_ref = TaskRef(thread.index);
        let task = self.kernel.task(task_ref).expect("invalid thread index");
        let entrypoint = task.entrypoint;
        let t = self
            .kernel
            .spawn_thread(task_ref, 0, usize::MAX, 0, entrypoint, thread.caps)
            .unwrap();

        self.idle_task_set = true;
        t
    }

    /// Adds a new [`abi::Endpoint`] capability to the specified task, pointing to the destination and address
    ///
    /// # Args
    /// `task` is the task to add the endpoint to.
    /// `dest` is the destination of the endpoint
    /// `addr` is the address for the endpoint, this is used to allow a single task to accept multiple message types
    pub fn endpoint(&mut self, task: ThreadRef, dest: ThreadRef, addr: usize) -> &mut Self {
        let _dest = self.kernel.scheduler.get_tcb(dest).unwrap();
        let task = self.kernel.scheduler.get_tcb_mut(task).unwrap();
        task.add_cap(Cap::Endpoint(Endpoint {
            tcb_ref: dest,
            addr,
            disposable: false,
        }));
        self
    }

    /// Starts the kernel
    pub fn start(self) -> ! {
        self.kernel.start()
    }
}

/// A builder for a thread, that can be passed into [`KernelBuilder`]
///
/// This struct will almost always be generated using the consts from the generated `task_table`
pub struct ThreadBuilder {
    index: usize,
    priority: usize,
    budget: usize,
    cooldown: usize,
    caps: List<CapEntry>,
    loans: heapless::Vec<RegionBuilder, 16>,
}

impl ThreadBuilder {
    /// Creates a new thread builder from a task index
    ///
    /// # Safety
    /// The index you pass in should be a valid thread_ref, this type is usually created by the codegen module
    /// This isn't truley "unsafe", but is marked as such to discourage use
    pub const unsafe fn new(index: usize) -> Self {
        Self {
            index,
            priority: 0,
            budget: usize::MAX,
            cooldown: 0,
            caps: List::new(),
            loans: heapless::Vec::new(),
        }
    }

    /// Set's the priority of a thread, value must be below 8.
    ///
    /// 7 is the highest priority and 0 is the lowest in k5
    pub fn priority(mut self, p: usize) -> Self {
        assert!(p < 8, "priority greater than 7");
        self.priority = p;
        self
    }
    /// Set's the budget of a thread, i.e the number of ticks before the thread yields
    ///
    /// Must be greater than 0
    pub fn budget(mut self, b: usize) -> Self {
        assert!(b > 0, "budget can not be zero");
        self.budget = b;
        self
    }

    /// Set's the cooldown of a thread.
    ///
    /// A thread's cooldown is the number of ticks before it get rescheduled after its budget is exhausted
    pub fn cooldown(mut self, c: usize) -> Self {
        self.cooldown = c;
        self
    }

    /// Adds a listen cap to the thread
    pub fn listen(mut self, port: PortId) -> Self {
        self.caps.push_back(Box::pin(CapEntry {
            cap: Cap::Listen(Listen { port }),
            _links: Default::default(),
        }));
        self
    }

    /// Adds a connect cap to the thread
    pub fn connect(mut self, port: PortId) -> Self {
        self.caps.push_back(Box::pin(CapEntry {
            cap: Cap::Connect(abi::Connect { port }),
            _links: Default::default(),
        }));
        self
    }

    pub fn loan_mem(mut self, region: RegionBuilder) -> Self {
        self.loans
            .push(region)
            .map_err(|_| KernelError::ABI(abi::Error::BufferOverflow))
            .unwrap();
        self
    }
}

pub struct RegionBuilder(Region);

impl RegionBuilder {
    pub fn new(range: Range<usize>, attr: BitFlags<RegionAttr>) -> Self {
        RegionBuilder(Region { range, attr })
    }
    pub fn device<T>(ptr: *const T) -> Self {
        let len = mem::size_of::<T>();
        RegionBuilder(Region {
            range: ptr.addr()..ptr.addr() + len,
            attr: RegionAttr::Device.into(),
        })
    }

    pub fn write(mut self) -> Self {
        self.0.attr |= RegionAttr::Write;
        self
    }

    pub fn read(mut self) -> Self {
        self.0.attr |= RegionAttr::Read;
        self
    }

    pub fn exec(mut self) -> Self {
        self.0.attr |= RegionAttr::Exec;
        self
    }

    pub fn dma(mut self) -> Self {
        self.0.attr |= RegionAttr::Dma;
        self
    }

    fn build(self) -> Region {
        self.0
    }
}

/// Creates a new mod called `task_table` with the generated task table from `k5-codegen`
#[macro_export]
macro_rules! include_task_table {
    () => {
        mod task_table {
            #![allow(dead_code)]
            include!(concat!(env!("OUT_DIR"), "/codegen.rs"));
        }
    };
}
