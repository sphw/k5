#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc_cortex_m::CortexMHeap;
use core::panic::PanicInfo;
use cortex_m_rt::entry;

mod task_table {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/codegen.rs"));
}

#[global_allocator]
static ALLOCATOR: CortexMHeap = CortexMHeap::empty();

#[entry]
fn main() -> ! {
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 0x1000;
        static mut HEAP: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { crate::ALLOCATOR.init(HEAP.as_ptr() as usize, HEAP_SIZE) }
    }
    let kernel =
        unsafe { kernel::arch::init_kernel(task_table::TASKS, task_table::TASK_IDLE_INDEX) };
    let tcb = kernel
        .scheduler
        .get_tcb_mut(abi::ThreadRef::idle())
        .unwrap();
    tcb.add_cap(abi::Capability::Endpoint(abi::Endpoint {
        tcb_ref: abi::ThreadRef(1),
        addr: 0,
    }));
    let foo_ref = kernel::TaskRef(task_table::TASK_FOO_INDEX);
    let task = kernel.task(foo_ref).unwrap();
    kernel
        .spawn_thread(foo_ref, 7, 10, 10, task.entrypoint)
        .unwrap();
    kernel.start();
}

#[alloc_error_handler]
fn oom(_: core::alloc::Layout) -> ! {
    loop {
        cortex_m::asm::bkpt();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        cortex_m::asm::bkpt();
    }
}
