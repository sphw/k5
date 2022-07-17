#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

#[export_name = "main"]
pub fn main() -> ! {
    let caps = userspace::get_caps().unwrap();
    defmt::println!("{:?}", &*caps);
    let mut buf = [0u8; 10];
    loop {
        match userspace::recv(0, &mut buf) {
            Ok(resp) => {
                defmt::println!("resp: {:?} buf: {:?}", resp, buf);
                if let Some(cap) = resp.cap {
                    buf[1..].copy_from_slice(&[0xA; 9]);
                    if let Err(err) = userspace::send_copy(cap, &mut buf) {
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

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
