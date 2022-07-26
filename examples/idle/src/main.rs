#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use defmt::info;
use userspace as _;

#[export_name = "main"]
pub fn main() -> ! {
    let mut a: u32 = 20;
    loop {
        a += 1;
        if a % 50000 == 0 {
            info!("idle")
        }
    }
}
