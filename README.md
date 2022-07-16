<img src = "./assets/logo.png" width="200px"/>

# K5 Microkernel

K5 is a very small microkernel based on the L4 family of kernels, but designed with microcontrollers in mind and written in Rust.

## Why

Why in the world does the world need another RTOS / microkernel. There are like half a dozen Rust RTOSes alone (Hubris, Tock, RTIC, MnemOS). There are a couple of good reasons, and a few "bad".

Let's start with the good. The largest reason is that this is a capability based microkernel. K5 most closely mimicks seL4 in its design. seL4 uses a concept called capabilities as access-control for IPC. The K5 scheduler is also based on seL4's MCS, though with some differences. Like seL4, but unlike many existing Rust RTOSes, K5 is *meant* to formally verified. This is not done yet, but is a primary goal of the kernel. The last major feature that differentiates K5 is that it is designed with support for enclaves, as first class citizens. I am not aware of any existing RTOS that supports scheduling TrustZone-M and RISC-V PMP enclaves natively. 

Now lets discuss the "bad" reasons. I've always been fascinated by kernel development, and have long wanted to try it out for myself. Of course I could have just contributed to an existing kernel, but that wouldn't be as fun. 

## What
Right now there is some basic functionality mainly for Cortex-M.

## Name

Ok, so to be honest the name was a happy-ish accident. I needed a random name, and I knew I was building an L4 kernel. So a number and letter sounded nice together. But I now have two nice stories to backup the name. The first is that much like k8s or i18n, the 5 is a stand in for the rest of the word "kernel". Also, mountains in the Karakoram region of a few countries (I'm not here to create geopolitical conflict), are surveyed starting with K. K5 is Gasherbrum 1, which serves as the inspiration for the logo.

## Credits
This projects owes a lot to seL4 and Hubris OS, both fantastic kernels and resources. The context-switching, syscall code, and build system are heavily inspired by Hubris. The scheduler and IPC system are heavily based on seL4.
