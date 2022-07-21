---
sidebar_position: 1
slug: /
---

# K5 Microkernel

K5 is a small microkernel closely related to the L4 family. It's niche is in embedded systems that need some element of security and/or safety guarenetee. Think industrial equipment, or secure elements. It currently runs on ARMv8m microcontrollers, and is being tested on the STM32L56. Eventually we plan to port K5 to other the other Cortex-M versions and RISC-V. 

## Getting Started

### Install CLI
The easiest way is by running 
```sh
cargo install --git https://github.com/sphw/k5.git --bins k5
```

### STM32L5 Example

Then go to `./examples/stm32l5` and run `k5 logs`. If everything goes right you should see something like below.

[![asciicast](https://asciinema.org/a/509730.svg)](https://asciinema.org/a/509730)

## Goals
#### Small readable code-base.

A primary goal of this project is to create an approchable microkernel. The more people can read and fully understand the code in K5, the less bugs there will be.

#### First-class developer experience
Embedded development is quite a mixed bag of tooling, some good, some terrible. K5's goal is to make every supported platform easy to work with. Thankfully the embedded Rust community has made this a lot easier with tools like [probe-rs](https://github.com/probe-rs/probe-rs) and [defmt](https://github.com/knurling-rs/defmt). K5 should also have best-in-class userspace libraries, to make it easy to develop new applications.

#### Employ formal verification methods and other static analysis tools everywhere possible.

Software engineering has long been the wild-west of the engineering world. There have been many attempts to improve this state-of-affairs, but they are often so cumbersome that they quickly become abandoned. Rust solves part of this problem through its approach to memory-safety, and there are other promising tools in the Rust eco-system that may allow us to verify large parts of the kernel. seL4 has led the way by formally verifying the entire kernel. In the short-term we plan to verify parts of K5's scheduler using [Kani](https://github.com/model-checking/kani).

#### Strong task isolation
Much like seL4, K5 utilizies a capability based system for security. One of K5's goals is to provide strong isolation between tasks, and to ensure that communication only occurs through proper channels. This helps limit the blast-radius of security vulnerabilities.

#### Native enclave support
In recent years enclave support has been addede to a whole variety of process. In particular TrustZone-M and PMP are becoming very common on microcontrollers. Current RTOSes leave it up to the user to figure out enclaves on their own, or they are told to use ARM TF-M (which is difficult to use and incomplete). K5 will provide enclave support for both RISC-V and ARMv8m, and make it a first class citizen on the OS. 

## Non-Goals

#### General purpose OS
K5 is not going to be Linux, Darwin, or even Fuschia / Zychron. The goal is to make a kernel for high-security embedded applications, not your laptop. Thankfully, that means we don't have to worry about a whole-host of issues that most OSes worry about.

#### Plug and play driver support

Have you ever used Zephyr? Its kinda crazy how you can almost seemlessly port code from one board to another with little effort. But have you ever tried to use Zephyr in a non-standard way, yikes. That is NOT K5's goal. We want a healthy eco-system of drivers, hopefully with standard-ized interfaces for common operations, but code-compatibility between unrelated devices is not the end-goal.
