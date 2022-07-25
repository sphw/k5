#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc_cortex_m::CortexMHeap;
use core::{mem::MaybeUninit, panic::PanicInfo};
use cortex_m_rt::entry;
use defmt::{error, info};

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
    let foo = kernel.thread(
        task_table::FOO
            .priority(7)
            .budget(2)
            .cooldown(5)
            .listen(*b"0123456789abcdef"),
    );
    let bar = kernel.thread(
        task_table::BAR
            .priority(7)
            .budget(2)
            .cooldown(5)
            .connect(*b"0123456789abcdef"),
    );
    kernel.endpoint(bar, foo, 0);
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
    error!("kernel panic: {:?}", defmt::Debug2Format(info));
    loop {
        cortex_m::asm::bkpt();
    }
}
