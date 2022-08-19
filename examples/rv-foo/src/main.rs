#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use userspace as _;

use core::arch::asm;

#[export_name = "main"]
pub fn main() -> ! {
    let mut a = 0;
    let mut b = 0;
    //let caps = userspace::caps().unwrap();
    let mut c = 0;
    loop {
        //let caps = userspace::caps().unwrap();
        userspace::println!("test: {:?}", a);
        a += 1;
        if a % 1_000_000 == 0 {
            b += 1;
            c += 2;
        }
    }
}
