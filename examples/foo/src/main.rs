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
            }
            Err(err) => {
                defmt::println!("syscall err");
            }
        }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
