#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use userspace::CapExt;

#[export_name = "main"]
pub fn main() -> ! {
    let caps = userspace::caps().unwrap();
    defmt::println!("{:?}", &*caps);
    caps[0].cap_ref.listen().unwrap();
    defmt::println!("listen");
    let mut buf = [0u8; 10];
    loop {
        match userspace::recv(0, &mut buf) {
            Ok(resp) => {
                defmt::println!("resp: {:?} buf: {:?}", resp, buf);
                if let Some(cap) = resp.cap {
                    buf[1..].copy_from_slice(&[0xA; 9]);
                    if let Err(err) = cap.send(&mut buf) {
                        defmt::error!("syscall err: {:?}", err);
                    }
                }
            }
            Err(err) => {
                defmt::error!("syscall err: {:?}", err);
            }
        }
    }
}
