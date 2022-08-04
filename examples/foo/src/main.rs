#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use stm32_hal2::clocks::Clocks;
use stm32_hal2::gpio::{Pin, PinMode, Port};
use userspace::CapExt;

#[export_name = "main"]
pub fn main() -> ! {
    let clock_cfg = Clocks::default();
    clock_cfg.setup().unwrap();
    let mut led = Pin::new(Port::D, 3, PinMode::Output);
    led.set_low();

    let caps = userspace::caps().unwrap();
    defmt::println!("{:?}", &*caps);
    caps[0].cap_ref.listen().unwrap();
    defmt::println!("listen");
    let mut buf = [0u8; 10];
    let mut toggle = false;
    loop {
        match userspace::recv::<_, [u8; 32]>(0, &mut buf) {
            Ok(resp) => {
                defmt::println!("resp: {:?} buf: {:?}", resp, buf);
                if let Some(cap) = resp.cap {
                    match resp.body {
                        userspace::RecvRespBody::Copy(_) => {
                            buf[1..].copy_from_slice(&[0xA; 9]);
                            if let Err(err) = cap.send(&mut buf) {
                                defmt::error!("syscall err: {:?}", err);
                            }
                        }
                        userspace::RecvRespBody::Page(mut buf) => {
                            defmt::println!("got slice: {:?}", buf);
                            buf[2..].copy_from_slice(&[0xA; 30]);
                            defmt::println!("wrote");
                            if let Err(err) = cap.send_page(buf) {
                                defmt::error!("syscall err: {:?}", err);
                            }
                        }
                    }
                    if toggle {
                        led.set_low();
                    } else {
                        led.set_high();
                    }
                    toggle = !toggle;
                }
            }
            Err(err) => {
                defmt::error!("syscall err: {:?}", err);
            }
        }
    }
}
