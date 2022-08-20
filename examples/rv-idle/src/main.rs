#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use core::arch::asm;
use userspace as _;
use userspace::{println, CapExt, Page};

#[export_name = "main"]
pub fn main() -> ! {
    let mut a: u32 = 20;
    loop {
        a += 1;
        if a % 50000 == 0 {
            println!("idle");
        }
    }
}
