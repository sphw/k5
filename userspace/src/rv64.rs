use abi::{SyscallArgs, SyscallIndex, SyscallReturn};
use core::arch::asm;

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

#[naked]
pub(crate) unsafe extern "C" fn syscall(
    index: SyscallIndex,
    args: &mut SyscallArgs,
) -> SyscallReturn {
    asm!(
        "
          ld a2, 0*8(a1)
          ld a3, 1*8(a1)
          ld a4, 2*8(a1)
          ld a5, 3*8(a1)
          ld a6, 4*8(a1)
          ld a7, 4*8(a1)
          ld a1, 0*8(a1)

          addi    sp,sp,-8 * 12
          sd s0,  0*8(sp)
          sd s1,  1*8(sp)
          sd s2,  2*8(sp)
          sd s3,  3*8(sp)
          sd s4,  4*8(sp)
          sd s5,  5*8(sp)
          sd s6,  6*8(sp)
          sd s7,  6*8(sp)
          sd s8,  7*8(sp)
          sd s9,  9*8(sp)
          sd s10, 10*8(sp)
          sd s11, 11*8(sp)

          ecall

          ld s0,  0*8(sp)
          ld s1,  1*8(sp)
          ld s2,  2*8(sp)
          ld s3,  3*8(sp)
          ld s4,  4*8(sp)
          ld s5,  5*8(sp)
          ld s6,  6*8(sp)
          ld s7,  6*8(sp)
          ld s8,  7*8(sp)
          ld s9,  9*8(sp)
          ld s10, 10*8(sp)
          ld s11, 11*8(sp)
          addi    sp,sp, 8 * 12
          ret
        ",
        options(noreturn)
    )
}
