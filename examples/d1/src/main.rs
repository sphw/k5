#![feature(naked_functions, asm_sym, asm_const)]
#![feature(alloc_error_handler)]
#![no_std]
#![no_main]

mod timer;

use core::{
    alloc::{GlobalAlloc, Layout},
    arch::asm,
    cell::RefCell,
    mem::MaybeUninit,
    ptr::NonNull,
};
use d1_pac::{PLIC, TIMER};
use linked_list_allocator::Heap;
use riscv::{
    interrupt::Mutex,
    register::mcause::{Exception, Interrupt, Trap},
};

#[global_allocator]
static ALLOCATOR: RISCVHeap = RISCVHeap::empty();

kernel::include_task_table! {}

mod de;

use crate::timer::{Timer, TimerMode, TimerPrescaler, TimerSource, Timers};

struct Uart(d1_pac::UART0);
static mut PRINTER: Option<Uart> = None;

#[no_mangle]
fn board_log(bytes: &[u8]) {
    let printer = unsafe { PRINTER.as_mut().unwrap() };
    for byte in bytes {
        printer.0.thr().write(|w| unsafe { w.thr().bits(*byte) });
        while printer.0.usr.read().tfnf().bit_is_clear() {}
    }
}

#[riscv_rt::entry]
fn main() -> ! {
    let p = d1_pac::Peripherals::take().unwrap();

    riscv::register::mscratch::write(0x0);

    // Enable UART0 clock.
    let ccu = &p.CCU;
    ccu.uart_bgr
        .write(|w| w.uart0_gating().pass().uart0_rst().deassert());

    // Set PC1 LED to output.
    let gpio = &p.GPIO;
    gpio.pc_cfg0
        .write(|w| w.pc1_select().output().pc0_select().ledc_do());

    // Set PB8 and PB9 to function 6, UART0, internal pullup.
    gpio.pb_cfg1
        .write(|w| w.pb8_select().uart0_tx().pb9_select().uart0_rx());
    gpio.pb_pull0
        .write(|w| w.pc8_pull().pull_up().pc9_pull().pull_up());

    // Configure UART0 for 115200 8n1.
    // By default APB1 is 24MHz, use divisor 13 for 115200.
    let uart0 = p.UART0;
    uart0.mcr.write(|w| unsafe { w.bits(0) });
    uart0.fcr().write(|w| w.fifoe().set_bit());
    uart0.halt.write(|w| w.halt_tx().enabled());
    uart0.lcr.write(|w| w.dlab().divisor_latch());
    uart0.dll().write(|w| unsafe { w.dll().bits(13) });
    uart0.dlh().write(|w| unsafe { w.dlh().bits(0) });
    uart0.lcr.write(|w| w.dlab().rx_buffer().dls().eight());
    uart0.halt.write(|w| w.halt_tx().disabled());
    unsafe { PRINTER = Some(Uart(uart0)) };

    {
        const HEAP_SIZE: usize = 0x1000;
        static mut HEAP: &mut [MaybeUninit<u8>; HEAP_SIZE] =
            &mut [MaybeUninit::uninit(); HEAP_SIZE];
        // Safety: we only ever access this once durring init, so this operation is safe
        crate::ALLOCATOR.init(unsafe { HEAP })
    }
    init_pmp();

    // // Set up timers
    // let Timers {
    //     mut timer0,
    //     mut timer1,
    //     ..
    // } = Timers::new(p.TIMER);

    // timer0.set_source(TimerSource::OSC24_M);
    // timer1.set_source(TimerSource::OSC24_M);

    // timer0.set_prescaler(TimerPrescaler::P8); // 24M / 8:  3.00M ticks/s
    // timer1.set_prescaler(TimerPrescaler::P32); // 24M / 32: 0.75M ticks/s

    // timer0.set_mode(TimerMode::SINGLE_COUNTING);
    // timer1.set_mode(TimerMode::SINGLE_COUNTING);

    // let _ = timer0.get_and_clear_interrupt();
    // let _ = timer1.get_and_clear_interrupt();

    unsafe {
        riscv::interrupt::enable();
        riscv::register::mie::set_mext();
        riscv::register::mie::set_usoft();
    }

    // yolo
    // timer0.set_interrupt_en(true);
    // timer1.set_interrupt_en(true);
    // let plic = &p.PLIC;

    // plic.prio[75].write(|w| w.priority().p1());
    // plic.prio[76].write(|w| w.priority().p1());
    // plic.mie[2].write(|w| unsafe { w.bits((1 << 11) | (1 << 12)) });

    // // Blink LED
    // loop {
    //     // Start both counters for 3M ticks: that's 1s for timer 0
    //     // and 4s for timer 1, for a 25% duty cycle
    //     timer0.start_counter(3_000_000);
    //     timer1.start_counter(3_000_000);
    //     gpio.pc_dat.write(|w| unsafe { w.bits(2) });

    //     unsafe { riscv::asm::wfi() };
    //     // while !timer0.get_and_clear_interrupt() { }

    //     gpio.pc_dat.write(|w| unsafe { w.bits(0) });
    //     unsafe { riscv::asm::wfi() };
    // }
    let mut kernel = kernel::KernelBuilder::new(task_table::TASKS);
    let _idle = kernel.idle_thread(task_table::IDLE.connect(*b"0123456789abcdef"));
    kernel.start()
}

pub struct RISCVHeap {
    heap: Mutex<RefCell<Heap>>,
}

impl RISCVHeap {
    pub const fn empty() -> RISCVHeap {
        RISCVHeap {
            heap: Mutex::new(RefCell::new(Heap::empty())),
        }
    }

    pub fn init(&self, mem: &'static mut [MaybeUninit<u8>]) {
        riscv::interrupt::free(move |cs| {
            self.heap.borrow(*cs).borrow_mut().init_from_slice(mem);
        });
    }
}

unsafe impl GlobalAlloc for RISCVHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        riscv::interrupt::free(|cs| {
            self.heap
                .borrow(*cs)
                .borrow_mut()
                .allocate_first_fit(layout)
                .ok()
                .map_or(core::ptr::null_mut(), |allocation| allocation.as_ptr())
        })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        riscv::interrupt::free(|cs| {
            self.heap
                .borrow(*cs)
                .borrow_mut()
                .deallocate(NonNull::new_unchecked(ptr), layout)
        });
    }
}

#[alloc_error_handler]
fn oom(_: core::alloc::Layout) -> ! {
    loop {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn init_pmp() {
    use riscv::register::*;
    // let cfg = 0x0f090f090fusize; // pmpaddr0-1 and pmpaddr2-3 are read-only
    // pmpcfg0::write(cfg);
    unsafe {
        riscv::register::pmpcfg0::set_pmp(0, Range::NAPOT, Permission::RWX, false);
        riscv::register::pmpcfg0::set_pmp(1, Range::NAPOT, Permission::RWX, false);
    }
    pmpcfg2::write(0); // nothing active here
    pmpaddr0::write(0x40000000usize >> 2 | (0xf00000 - 1));
    pmpaddr1::write(0x40f00000usize >> 2 | (0xf00000 - 1));
    // pmpaddr1::write(0x40200000usize >> 2);
    pmpaddr2::write(0x80000000usize >> 2);
    pmpaddr3::write(0x80200000usize >> 2);
    // pmpaddr4::write(0xffffffffusize >> 2);
}
