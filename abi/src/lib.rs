#![no_std]

use core::ops::Deref;

use defmt::Format;
use mycelium_bitfield::FromBits;

mycelium_bitfield::bitfield! {
    #[derive(Eq, PartialEq)] // ...and attributes
    pub struct SyscallIndex<u32> {
        pub const SYSCALL_FN: SyscallFn;
        pub const SYSCALL_ARG_TYPE: SyscallDataType;
    }
}

#[derive(Debug)]
#[repr(u8)]
pub enum SyscallFn {
    Send = 0b001,
    Call = 0b010,
    Recv = 0b011,
    Log = 0b0101,
    Caps = 0b111,
}

impl FromBits<u32> for SyscallFn {
    const BITS: u32 = 3;
    type Error = &'static str;

    fn try_from_bits(bits: u32) -> Result<Self, Self::Error> {
        match bits as u8 {
            bits if bits == Self::Send as u8 => Ok(Self::Send),
            bits if bits == Self::Call as u8 => Ok(Self::Call),
            bits if bits == Self::Recv as u8 => Ok(Self::Recv),
            bits if bits == Self::Log as u8 => Ok(Self::Log),
            bits if bits == Self::Caps as u8 => Ok(Self::Caps),
            _ => Err("expected valid syscall fn identifier"),
        }
    }

    fn into_bits(self) -> u32 {
        self as u8 as u32
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq)]
pub enum SyscallDataType {
    Short,
    Copy,
    Page,
}

impl FromBits<u32> for SyscallDataType {
    const BITS: u32 = 2;
    type Error = &'static str;

    fn try_from_bits(bits: u32) -> Result<Self, Self::Error> {
        match bits as u8 {
            bits if bits == Self::Short as u8 => Ok(Self::Short),
            bits if bits == Self::Copy as u8 => Ok(Self::Copy),
            bits if bits == Self::Page as u8 => Ok(Self::Page),
            _ => Err("expected valid syscall fn identifier"),
        }
    }

    fn into_bits(self) -> u32 {
        self as u8 as u32
    }
}

#[derive(Default)]
#[repr(C)]
pub struct SyscallArgs {
    pub arg1: usize,
    pub arg2: usize,
    pub arg3: usize,
    pub arg4: usize,
    pub arg5: usize,
    pub arg6: usize,
}

#[repr(u8)]
#[derive(Debug)]
pub enum SyscallReturnType {
    Error,
    Short,
    Page,
    Copy,
}

mycelium_bitfield::bitfield! {
    #[derive( Eq, PartialEq)]
    pub struct SyscallReturn<u64> {
        pub const SYSCALL_TYPE: SyscallReturnType;
        pub const SYSCALL_CAP = 8;
        pub const SYSCALL_LEN = 22;
        pub const SYSCALL_PTR = 32;
    }
}

impl SyscallReturn {
    pub fn split(self) -> (u32, u32) {
        ((self.0 >> 32) as u32, self.0 as u32)
    }
}

impl From<Error> for SyscallReturn {
    #[inline]
    fn from(err: Error) -> Self {
        SyscallReturn::new()
            .with(SyscallReturn::SYSCALL_TYPE, SyscallReturnType::Error)
            .with(SyscallReturn::SYSCALL_LEN, (u8::from(err)) as u64)
    }
}

impl FromBits<u64> for SyscallReturnType {
    const BITS: u32 = 2;
    type Error = &'static str;

    fn try_from_bits(bits: u64) -> Result<Self, &'static str> {
        match bits as u8 {
            bits if bits == Self::Error as u8 => Ok(Self::Error),
            bits if bits == Self::Short as u8 => Ok(Self::Short),
            bits if bits == Self::Copy as u8 => Ok(Self::Copy),
            bits if bits == Self::Page as u8 => Ok(Self::Page),
            _ => Err("expected valid syscall fn identifier"),
        }
    }

    fn into_bits(self) -> u64 {
        self as u8 as u64
    }
}

#[derive(Debug)]
#[repr(u8)]
pub enum Error {
    ReturnTypeMismatch,
    BadAccess,
    BufferOverflow,
    Unknown(u8),
}

impl From<u8> for Error {
    fn from(code: u8) -> Self {
        match code {
            1 => Error::ReturnTypeMismatch,
            2 => Error::BadAccess,
            3 => Error::BufferOverflow,
            code => Error::Unknown(code),
        }
    }
}

impl From<Error> for u8 {
    fn from(err: Error) -> Self {
        match err {
            Error::ReturnTypeMismatch => 1,
            Error::BadAccess => 2,
            Error::BufferOverflow => 3,
            Error::Unknown(code) => code,
        }
    }
}

#[derive(Clone, Copy, defmt::Format)]
pub struct CapabilityRef(pub usize);
impl Deref for CapabilityRef {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<usize> for CapabilityRef {
    fn from(i: usize) -> Self {
        CapabilityRef(i)
    }
}

#[derive(Clone, defmt::Format)]
#[repr(C)]
pub enum Capability {
    Endpoint(Endpoint),
    Notification,
}

#[repr(C)]
#[derive(Clone, Copy, defmt::Format)]
pub struct Endpoint {
    pub tcb_ref: ThreadRef,
    pub addr: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, defmt::Format)]
#[repr(C)]
pub struct ThreadRef(pub usize);

impl Deref for ThreadRef {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ThreadRef {
    pub const fn idle() -> ThreadRef {
        ThreadRef(0)
    }
}

#[derive(Format)]
pub struct CapListEntry {
    pub cap_ref: CapabilityRef,
    pub desc: Capability,
}

// LOGING
// Basic system will piggy back almost fully on defmt
// demft stores all their log info in an elf
// there is 1 elf per task
// we will send logs to the kernel which will send them to rtt with the associated task index
// the parser will get the first byte, holding the index, and then parse the rest of the log using the
// appropriate symbol table
