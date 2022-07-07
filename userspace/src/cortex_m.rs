use core::{arch::asm, mem::MaybeUninit, ops::Deref};

use abi::{
    Capability, CapabilityRef, Error, SyscallArgs, SyscallDataType, SyscallFn, SyscallIndex,
    SyscallReturn, SyscallReturnType,
};

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
unsafe extern "C" fn syscall(index: SyscallIndex, args: &mut SyscallArgs) -> SyscallReturn {
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

pub fn send_page<T: ?Sized>(capability: CapabilityRef, r: &mut T) -> Result<(), Error> {
    let size = core::mem::size_of_val(r);
    let (ptr, _) = (r as *mut T).to_raw_parts();
    let addr = ptr.addr();
    let index = SyscallIndex::new()
        .with(SyscallIndex::SYSCALL_ARG_TYPE, SyscallDataType::Page)
        .with(SyscallIndex::SYSCALL_FN, SyscallFn::Send)
        .with(SyscallIndex::CAPABILITY, *capability as u32);
    let mut args = SyscallArgs {
        arg1: addr,
        arg2: size,
        ..Default::default()
    };
    let res = unsafe { syscall(index, &mut args) };
    match res.get(SyscallReturn::SYSCALL_TYPE) {
        SyscallReturnType::Error => {
            let code = res.get(SyscallReturn::SYSCALL_LEN);
            return Err(abi::Error::from(code as u8));
        }
        _ => {}
    }
    Ok(())
}

pub fn get_caps() -> Result<CapList, Error> {
    const ELEM: MaybeUninit<Capability> = MaybeUninit::uninit();
    let index = SyscallIndex::new()
        .with(SyscallIndex::SYSCALL_ARG_TYPE, SyscallDataType::Short)
        .with(SyscallIndex::SYSCALL_FN, SyscallFn::Caps)
        .with(SyscallIndex::CAPABILITY, 0);
    let mut buf = [ELEM; 10];
    let ptr = buf.as_mut_ptr();
    let mut args = SyscallArgs {
        arg1: ptr.addr(),
        arg2: buf.len(),
        ..Default::default()
    };
    let res = unsafe { syscall(index, &mut args) };
    match res.get(SyscallReturn::SYSCALL_TYPE) {
        SyscallReturnType::Error => {
            let code = res.get(SyscallReturn::SYSCALL_LEN);
            Err(abi::Error::from(code as u8))
        }
        SyscallReturnType::Copy => {
            let len = res.get(SyscallReturn::SYSCALL_LEN) as usize;
            Ok(CapList { buf, len })
        }
        _ => Err(abi::Error::ReturnTypeMismatch),
    }
}

pub struct CapList {
    buf: [MaybeUninit<Capability>; 10],
    len: usize,
}

impl Deref for CapList {
    type Target = [Capability];

    fn deref(&self) -> &Self::Target {
        // Safety:
        // The core invarient here is that when you create `CapList`
        // you ensure that 0..len items are initialized
        unsafe { core::mem::transmute(&self.buf[0..self.len]) }
    }
}
