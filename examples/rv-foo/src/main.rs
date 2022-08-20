#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use core::arch::asm;
use userspace as _;
use userspace::CapExt;

#[export_name = "main"]
pub fn main() -> ! {
    let caps = userspace::caps().unwrap();
    userspace::println!("{:?}", &*caps);
    caps[0].cap_ref.listen().unwrap();
    userspace::println!("listen");
    let mut buf = [0u8; 10];
    loop {
        userspace::println!("recv");
        match userspace::recv::<_, [u8; 32]>(0, &mut buf) {
            Ok(resp) => {
                userspace::println!("resp: {:?} buf: {:?}", resp, buf);
                if let Some(cap) = resp.cap {
                    match resp.body {
                        userspace::RecvRespBody::Copy(_) => {
                            buf[1..].copy_from_slice(&[0xA; 9]);
                            if let Err(err) = cap.send(&mut buf) {
                                userspace::println!("syscall err: {:?}", err);
                            }
                        }
                        userspace::RecvRespBody::Page(mut buf) => {
                            userspace::println!("got slice: {:?}", buf);
                            buf[2..].copy_from_slice(&[0xA; 30]);
                            userspace::println!("wrote");
                            if let Err(err) = cap.send_page(buf) {
                                userspace::println!("syscall err: {:?}", err);
                            }
                        }
                    }
                }
            }
            Err(err) => {
                userspace::println!("syscall err: {:?}", err);
            }
        }
    }
}
