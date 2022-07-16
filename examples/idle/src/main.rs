#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use defmt::println;

#[export_name = "main"]
pub fn main() -> ! {
    let caps = userspace::get_caps().unwrap();
    println!("{:?}", &*caps);
    let mut a: u32 = 20;
    loop {
        a += 1;
        if a % 500000 == 0 {
            println!("send");
            userspace::send_copy(0.into(), &mut [0xFFu8; 10]);
        }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
