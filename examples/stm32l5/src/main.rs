#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc_cortex_m::CortexMHeap;
use core::{mem::MaybeUninit, panic::PanicInfo};
use cortex_m_rt::{entry, exception};
use defmt::{error, info};
use kernel::{RegionAttr, RegionBuilder};
use stm32l5::stm32l562;

kernel::include_task_table! {}

#[global_allocator]
static ALLOCATOR: CortexMHeap = CortexMHeap::empty();

#[entry]
fn main() -> ! {
    {
        const HEAP_SIZE: usize = 0x1000;
        static mut HEAP: &mut [MaybeUninit<u8>; HEAP_SIZE] =
            &mut [MaybeUninit::uninit(); HEAP_SIZE];
        // Safety: we only ever access this once durring init, so this operation is safe
        crate::ALLOCATOR.init(unsafe { HEAP })
    }

    let mut kernel = kernel::KernelBuilder::new(task_table::TASKS);
    let _idle = kernel.idle_thread(task_table::IDLE);

    let bar_thread = kernel.thread(
        task_table::BAR
            .priority(7)
            .budget(100)
            .cooldown(50)
            .connect(*b"0123456789abcdef"),
    );
    let foo_thread = kernel.thread(
        task_table::FOO
            .priority(7)
            .budget(5)
            .cooldown(usize::MAX)
            .loan_mem(
                RegionBuilder::new(0x4000_1000..0x4202fc00, RegionAttr::Device.into())
                    .write()
                    .read(),
            )
            //.loan_mem(RegionBuilder::device(stm32l562::RCC::PTR).write().read())
            //.loan_mem(RegionBuilder::device(stm32l562::GPIOA::PTR).write().read())
            // .loan_mem(RegionBuilder::device(stm32l562::GPIOD::PTR).write().read())
            // .loan_mem(RegionBuilder::device(stm32l562::GPIOG::PTR).write().read())
            //.loan_mem(RegionBuilder::device(stm32l562::PWR::PTR).write().read())
            //.loan_mem(RegionBuilder::device(stm32l562::FLASH::PTR).write().read())
            .listen(*b"0123456789abcdef"),
    );

    kernel.endpoint(bar_thread, foo_thread, 0);
    info!("booting");
    kernel.start()
}

#[alloc_error_handler]
fn oom(_: core::alloc::Layout) -> ! {
    error!("kernel out of memory");
    loop {
        cortex_m::asm::bkpt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    error!("kern panic: {}", defmt::Display2Format(info));
    loop {
        cortex_m::asm::bkpt();
    }
}

#[exception]
unsafe fn HardFault(ef: &cortex_m_rt::ExceptionFrame) -> ! {
    defmt::println!("{:?}", defmt::Debug2Format(ef));
    defmt::println!(
        "MemFault reg {:b}",
        core::ptr::read_volatile(0xE000ED28 as *const u16)
    );
    defmt::println!(
        "MemFault addr: {:x}",
        core::ptr::read_volatile(0xE000ED34 as *const u32)
    );
    defmt::println!(
        "UsageFault reg {:b}",
        core::ptr::read_volatile(0xE000ED2A as *const u16)
    );

    loop {}
}
