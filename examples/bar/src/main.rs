#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use defmt::{info, println};
use userspace::{CapExt, Page};

#[export_name = "main"]
pub fn main() -> ! {
    let caps = userspace::caps().unwrap();
    println!("{:?}", &*caps);
    let endpoint = caps[0].cap_ref.connect().unwrap();
    println!("connected: {:?}", endpoint);
    let mut a: u32 = 20;
    loop {
        a += 1;
        if a % 100000 == 0 {
            let mut buf = Page([0xFFu8; 32]);
            buf.0[0..4].copy_from_slice(&a.to_be_bytes());
            info!("send {:?} {:x}", buf.0, buf.0.as_ptr());
            let resp = endpoint.call_io(&mut buf);
            info!("resp {:?}, buf {:?}", resp, buf.0);
        }
    }
}
