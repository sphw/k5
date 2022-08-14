#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_sym)]

use core::arch::asm;

//use userspace::CapExt;
//

#[export_name = "main"]
pub fn main() -> ! {
    let mut a = 1;
    loop {
        a += 0xFF;
        unsafe { core::arch::asm!("ecall") };
        if a % 20 == 5 {
            a -= 1;
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[doc(hidden)]
#[no_mangle]
#[link_section = ".text.start"]
#[naked]
pub unsafe extern "C" fn _start() -> ! {
    // Provided by the user program:
    extern "Rust" {
        fn main() -> !;
    }

    asm!("
        # Copy data initialization image into data section.
        la t0, _edata       # upper bound in t0
        la t1, _sidata      # source in t1
        la t2, _sdata       # dest in t2
        j 1f
    2:  ld s3, (t1)
        add t1, t1, 4
        sd s3, (t2)
        add t2, t2, 4
    1:  bne t2, t0, 2b
        # Zero BSS
        la t0, _ebss        # upper bound in t0
        la t1, _sbss        # base in t1
        j 1f
    2:  sd zero, (t1)
        add t1, t1, 4
    1:  bne t1, t0, 2b
        j {main}
        ",
        main = sym main,
        options(noreturn),
    )
}
