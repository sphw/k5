use abi::{Capability, Endpoint, ThreadRef};

use crate::{Kernel, TaskDesc, TaskRef};

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

    /// Set's the cycles per tick
    pub fn cycles_per_tick(&mut self, cycles: usize) -> &mut Self {
        self.cycles_per_tick = cycles;
        self
    }

    pub fn thread(&mut self, thread: ThreadBuilder) -> ThreadRef {
        let task_ref = TaskRef(thread.index);
        let task = self.kernel.task(task_ref).expect("invalid thread index");
        self.kernel
            .spawn_thread(
                task_ref,
                thread.priority,
                thread.budget,
                thread.cooldown,
                task.entrypoint,
            )
            .unwrap()
    }

    pub fn idle_thread(&mut self, thread: ThreadBuilder) -> ThreadRef {
        let task_ref = TaskRef(thread.index);
        let task = self.kernel.task(task_ref).expect("invalid thread index");
        let t = self
            .kernel
            .spawn_thread(
                task_ref,
                thread.priority,
                thread.budget,
                thread.cooldown,
                task.entrypoint,
            )
            .unwrap();

        self.idle_task_set = true;
        t
    }

    pub fn start(self) -> ! {
        self.kernel.start()
    }

    pub fn attach_endpoint(&mut self, task: ThreadRef, dest: ThreadRef, addr: usize) -> &mut Self {
        let _dest = self.kernel.scheduler.get_tcb(dest).unwrap();
        let task = self.kernel.scheduler.get_tcb_mut(task).unwrap();
        task.add_cap(Capability::Endpoint(Endpoint {
            tcb_ref: dest,
            addr,
        }));
        self
    }
}

pub struct ThreadBuilder {
    index: usize,
    priority: usize,
    budget: usize,
    cooldown: usize,
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
}

#[macro_export]
macro_rules! include_task_table {
    () => {
        mod task_table {
            #![allow(dead_code)]
            include!(concat!(env!("OUT_DIR"), "/codegen.rs"));
        }
    };
}
