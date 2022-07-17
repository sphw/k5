<img src = "./docs/assets/logo.png" width="200px"/>

# K5 Microkernel

K5 is a very small microkernel based on the L4 family of kernels, but designed with microcontrollers in mind and written in Rust.

## Why

Why in the world does the world need another RTOS / microkernel. There are like half a dozen Rust RTOSes alone (Hubris, Tock, RTIC, MnemOS). There are a couple of good reasons, and a few "bad".

Let's start with the good. The largest reason is that this is a capability based microkernel. K5 most closely mimicks seL4 in its design. seL4 uses a concept called capabilities as access-control for IPC. The K5 scheduler is also based on seL4's MCS, though with some differences. Like seL4, but unlike many existing Rust RTOSes, K5 is *meant* to formally verified. This is not done yet, but is a primary goal of the kernel. The last major feature that differentiates K5 is that it is designed with support for enclaves, as first class citizens. I am not aware of any existing RTOS that supports scheduling TrustZone-M and RISC-V PMP enclaves natively. 

Now lets discuss the "bad" reasons. I've always been fascinated by kernel development, and have long wanted to try it out for myself. Of course I could have just contributed to an existing kernel, but that wouldn't be as fun. 

## What
Right now there is some basic functionality mainly for ARMv8M. There is an example project for the STM32L5, though it *should* work on any v8m CPU with a little modification. You can send and recieve messages between threads, the scheduler will pre-empt your tasks at set intervels, and there is a built in logging framework using defmt.

### Dir Layout
- `./kernel` - Contains the kernel as a library that will be used by a host application
- `./userspace` - The userspace library, contains functions for syscalls, startup, and logging
- `./abi` - The ABI (application binary interface) includes shared data-structures between the kernel and userspace
- `./codegen` - Simple code-generation utility to take a list of tasks, and produce Rust
- `./examples` - Example "apps" for various boards, right now just stm32l5.
- `./cli` - Contains the `k5` build tool, it supports flashing, building, and printing logs from a k5 app


## What's Next

I've got plans! So so many plans... Basically I think the order of tasks will be as follows
- [ ] Finish the initial set of syscalls (recv, call, send, logs, caps)
- [ ] Write MPU region solver and add MPU support for Cortex-M
- [ ] Make the UX better for the kernels APIs. Basically make starting the kernel and threads easier.
- [ ] Document both the code, and the design choices of the project
- [ ] Attempt to use Kani or Creusot to verify the base scheduler
- [ ] Port to RISC-V ?
- [ ] Add support for building and using TZ / PMP enclave tasks
- [ ] Investiage porting to MMU based systems

The order is subject to change based on what I feel like is the shiniest object, but that's the basic roadmap. If any of those things sound interesting to you and you want to help out, please reach out.


## Name

Ok, so to be honest the name was a happy-ish accident. I needed a random name, and I knew I was building an L4 kernel. So a number and letter sounded nice together. But I now have two nice stories to backup the name. The first is that much like k8s or i18n, the 5 is a stand in for the rest of the word "kernel". Also, mountains in the Karakoram region of a few countries (I'm not here to create geopolitical conflict), are surveyed starting with K. K5 is Gasherbrum 1, which serves as the inspiration for the logo.

## Credits
This projects owes a lot to seL4 and Hubris OS, both fantastic kernels and resources. The context-switching, syscall code, and build system are heavily inspired by Hubris. The scheduler and IPC system are heavily based on seL4. The logging system is just straight up defmt, but the RTT block is accessed through the kernel. Huge thanks to the Ferrous systems and the rest of the Rust embedded community, who have helped make embedded development far friendly to work with. 
