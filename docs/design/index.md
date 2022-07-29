---
sidebar_position: 2
slug: /design
---

# Design 

K5's architecture is designed to be easy to understand and port to a variety of architectures. As such it is broken into a few components. At a high level the kernel is made up of a few discrete components:
- Scheduler
- Syscall handlers
- Platform specific arch
- Registry
- Logger

This page will give an overview of each component, more details can be found in the sub page for each component.

## Scheduler

K5's scheduler is heavily based on the MCS system from seL4, which is intern based on ["Scheduling-Context Capabilities"](https://trustworthy.systems/publications/csiro_full_text/Lyons_MAH_18.pdf) by Anna Lyons, et all. There are some minor, but important, differences between MCS and k5 system. Both schedulers attempt to solve the problem of a high-priority task monopolizing CPU-time, and they solve this problem in fundemntally the same way.

In K5 each task is given a priority, a budget, and a cooldown. Threads are scheduled in a round-robin fashion in decending order of priority (7 is the highest, while 0 is the lowest). When a thread is first scheduled on the CPU, it is given an amount of time it is allowed to execute, its budget. Each tick this budget is reduced, when it reaches zero the thread is "exhausted" and execution is paused. It is then added to queue of exhausted threads. Each tick the exhausted list is scanned for threads whose cooldown has elapsed, once a cooldown is elapsed the thread is rescheduled. This technique is almost identical to MCS with some terminology differences. MCS has a "period" which is equivalent to k5's `budget + cooldown`. 

In seL4 MCS "cpu time" is tracked through capabilities, as is almost everything in seL4. This is because capabilities in seL4 are more general objects that can be shared by kernel and task. In K5 a capability are almost always used for soley syscalls. We still have internal objects for tracking CPU time per task in the scheduler, they just aren't also capabilities. 


Both K5 and MCS allow the user to "loan" out their CPU time to another task. In K5 this is done exclusively through the `call` syscall, which sends a message to another thread and waits for a response. When `call` is executed the current threads budget is loaned out to the receiving task, and execution is immedietly transfered. This allows you to implement "passive-servers", threads that lack their own budget's and are only ever scheduled when invoked by a client.
