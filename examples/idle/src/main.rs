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
        if a % 5000 == 0 {
            let mut buf = [0xFFu8; 10];
            buf[0..4].copy_from_slice(&a.to_be_bytes());
            println!("send {:?}", buf);
            let mut resp_buf = [0; 10];
            let resp = userspace::call_copy(caps[0].cap_ref, &mut buf, &mut resp_buf);
            println!("resp {:?}, buf: {:?}", resp, resp_buf);
        }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
