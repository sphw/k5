use abi::{
    CapListEntry, CapRef, Error, SyscallArgs, SyscallDataType, SyscallFn, SyscallIndex,
    SyscallReturn, SyscallReturnType,
};
use core::fmt::Write;
use core::mem;
use core::{
    arch::asm,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
};
use defmt::Format;

#[doc(hidden)]
#[no_mangle]
#[link_section = ".text.start"]
#[naked]
pub unsafe extern "C" fn _start() -> ! {
    extern "Rust" {
        fn main() -> !;
    }
    // pulled from
    asm!("
        @ copy data image into data section, this asumes that
        @ the source and destination are 32 bit aligned
        movw r0, #:lower16:__edata  @ upper bound in r0
        movt r0, #:upper16:__edata

        movw r1, #:lower16:__sidata @ source in r1
        movt r1, #:upper16:__sidata

        movw r2, #:lower16:__sdata  @ dest in r2
        movt r2, #:upper16:__sdata

        b 1f                        @ check for zero-sized data

    2:  ldr r3, [r1], #4            @ read and advance source
        str r3, [r2], #4            @ write and advance dest

    1:  cmp r2, r0                  @ has dest reached the upper bound?
        bne 2b                      @ if not, repeat

        @ Zero BSS section.

        movw r0, #:lower16:__ebss   @ upper bound in r0
        movt r0, #:upper16:__ebss

        movw r1, #:lower16:__sbss   @ base in r1
        movt r1, #:upper16:__sbss

        movs r2, #0                 @ materialize a zero

        b 1f                        @ check for zero-sized BSS

    2:  str r2, [r1], #4            @ zero one word and advance

    1:  cmp r1, r0                  @ has base reached bound?
        bne 2b                      @ if not, repeat

        @ Be extra careful to ensure that those side effects are
        @ visible to the user program.

        dsb         @ complete all writes
        isb         @ and flush the pipeline
        bl {main}
        ",
        main = sym main,
        options(noreturn)
    )
}

#[naked]
pub(crate) unsafe extern "C" fn syscall(
    index: SyscallIndex,
    args: &mut SyscallArgs,
) -> SyscallReturn {
    asm!(
        "
        push {{r4-r11}} @ push registers onto stack
        ldm r1, {{r4-r10}} @ load args from args struct
        mov r11, r0 @ load index into r0
        svc #0 @ trigger svc interrupt
        mov r0, r4 @ move results to return position
        mov r1, r5
        pop {{r4-r11}} @ restore registers
        bx lr
        ",
        options(noreturn)
    )
}
