#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use userspace as _;

use core::arch::asm;

#[export_name = "main"]
pub fn main() -> ! {
    let mut a = 1;
    loop {
        a += 0xFF;
        defmt::println!("test: {:?}", a);
        if a % 20 == 5 {
            a -= 1;
        }
    }
}
