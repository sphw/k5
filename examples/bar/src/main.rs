#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use defmt::{info, println};
use userspace::CapExt;

#[export_name = "main"]
pub fn main() -> ! {
    let caps = userspace::caps().unwrap();
    println!("{:?}", &*caps);
    let mut a: u32 = 20;
    loop {
        a += 1;
        if a % 50000 == 0 {
            let mut buf = [0xFFu8; 10];
            buf[0..4].copy_from_slice(&a.to_be_bytes());
            info!("send {:?}", buf);
            let mut resp_buf = [0; 10];
            let resp = caps[0].cap_ref.call(&mut buf, &mut resp_buf);
            info!("resp {:?}, buf: {:?}", resp, resp_buf);
        }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
