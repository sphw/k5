#![no_std]
#![feature(naked_functions)]
#![feature(strict_provenance)]
#![feature(ptr_metadata)]
#![feature(asm_sym)]

#[cfg(feature = "cortex_m")]
mod cortex_m;
#[cfg(feature = "cortex_m")]
pub use cortex_m::*;

#[cfg(feature = "rv64")]
mod rv64;
#[cfg(feature = "rv64")]
pub use rv64::*;

pub use abi;

mod defmt_logger;

use ::defmt::Format;
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

#[inline]
fn send_inner<T: ?Sized>(ty: SyscallDataType, capability: CapRef, r: &mut T) -> Result<(), Error> {
    let size = core::mem::size_of_val(r);
    let (ptr, _) = (r as *mut T).to_raw_parts();
    let addr = ptr.addr();
    let index = SyscallIndex::new()
        .with(SyscallIndex::SYSCALL_ARG_TYPE, ty)
        .with(SyscallIndex::SYSCALL_FN, SyscallFn::Send);
    let mut args = SyscallArgs {
        arg1: addr,
        arg2: size,
        arg3: *capability,
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

#[inline]
fn call_innner<T: ?Sized>(
    ty: SyscallDataType,
    capability: CapRef,
    r: &mut T,
    out: Option<&mut T>,
) -> Result<abi::RecvResp, Error> {
    let size = core::mem::size_of_val(r);
    let (ptr, _) = (r as *mut T).to_raw_parts();
    let addr = ptr.addr();
    let index = SyscallIndex::new()
        .with(SyscallIndex::SYSCALL_ARG_TYPE, ty)
        .with(SyscallIndex::SYSCALL_FN, SyscallFn::Call);
    let mut resp: MaybeUninit<abi::RecvResp> = MaybeUninit::uninit();
    let (out_size, out_addr) = if let Some(out) = out {
        let (ptr, _) = (out as *mut T).to_raw_parts();
        (core::mem::size_of_val(out), ptr.addr())
    } else {
        (0, 0)
    };

    let mut args = SyscallArgs {
        arg1: addr,
        arg2: size,
        arg3: *capability,
        arg4: resp.as_mut_ptr().addr(),
        arg5: out_addr,
        arg6: out_size,
    };
    let res = unsafe { syscall(index, &mut args) };
    match res.get(SyscallReturn::SYSCALL_TYPE) {
        SyscallReturnType::Error => {
            let code = res.get(SyscallReturn::SYSCALL_LEN);
            Err(abi::Error::from(code as u8))
        }
        _ => Ok(unsafe { resp.assume_init() }),
    }
}

#[inline]
pub(crate) fn log(data: &[u8]) -> Result<(), Error> {
    let (ptr, _) = data.as_ptr().to_raw_parts();
    let addr = ptr.addr();
    let index = SyscallIndex::new()
        .with(SyscallIndex::SYSCALL_ARG_TYPE, SyscallDataType::Copy)
        .with(SyscallIndex::SYSCALL_FN, SyscallFn::Log);
    let mut args = SyscallArgs {
        arg1: addr,
        arg2: data.len(),
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

pub trait CapExt {
    /// Sends a request to the capability and waits for a reply
    fn call<T: ?Sized>(&self, request: &mut T, out_buf: &mut T) -> Result<RecvResp<T>, Error>;

    /// Sends a request to the capability loaning the data in the io buf and waits for a reply
    fn call_io<'a, A: Aligned + 'a>(&self, io: &'a mut A) -> Result<(), Error>;

    /// Sends a request to the capability, and returns ASAP
    fn send<T: ?Sized>(&self, request: &mut T) -> Result<(), Error>;

    /// Sends a request to the capability, and returns ASAP
    fn send_page<A: Aligned + 'static>(&self, request: A) -> Result<(), Error>;

    /// Listens to the port on the specified capability
    fn listen(&self) -> Result<(), Error>;
    /// Connects to the port, and returns an endpoint one can second messages to
    fn connect(&self) -> Result<CapRef, Error>;
}

impl CapExt for CapRef {
    fn call<T: ?Sized>(&self, r: &mut T, out_buf: &mut T) -> Result<RecvResp<T>, Error> {
        let resp = call_innner(SyscallDataType::Copy, *self, r, Some(out_buf))?;
        if let abi::RecvRespInner::Copy(len) = resp.inner {
            Ok(RecvResp {
                cap: resp.cap,
                body: RecvRespBody::Copy(len),
            })
        } else {
            Err(Error::ReturnTypeMismatch)
        }
    }

    fn call_io<'a, A: Aligned + 'a>(&self, io: &'a mut A) -> Result<(), Error> {
        match call_innner(SyscallDataType::Page, *self, io, None)?.inner {
            abi::RecvRespInner::Copy(_) => {
                return Err(Error::ReturnTypeMismatch);
            }
            abi::RecvRespInner::Page { addr, len } => {
                // Since we are taking a mutable borrow over just the course of the syscall, we must guarentee
                // that we are getting back the same memory
                let (ptr, _) = (io.deref_mut() as *mut A::Target).to_raw_parts();
                if addr != ptr.addr() || mem::size_of_val(io.deref_mut()) != len {
                    defmt::error!("addr mismatch");
                    return Err(Error::ReturnTypeMismatch);
                }
            }
        }
        Ok(())
    }

    fn send<T: ?Sized>(&self, r: &mut T) -> Result<(), Error> {
        send_inner(SyscallDataType::Copy, *self, r)
    }

    fn send_page<A: Aligned + 'static>(&self, mut request: A) -> Result<(), Error> {
        send_inner::<A::Target>(SyscallDataType::Page, *self, request.deref_mut())
    }

    fn listen(&self) -> Result<(), Error> {
        let index = SyscallIndex::new().with(SyscallIndex::SYSCALL_FN, SyscallFn::Listen);
        let mut args = SyscallArgs {
            arg1: self.0,
            ..Default::default()
        };
        let res = unsafe { syscall(index, &mut args) };
        match res.get(SyscallReturn::SYSCALL_TYPE) {
            SyscallReturnType::Error => {
                let code = res.get(SyscallReturn::SYSCALL_LEN);
                Err(abi::Error::from(code as u8))
            }
            SyscallReturnType::Copy => Ok(()),
            _ => Err(abi::Error::ReturnTypeMismatch),
        }
    }

    fn connect(&self) -> Result<CapRef, Error> {
        let index = SyscallIndex::new().with(SyscallIndex::SYSCALL_FN, SyscallFn::Connect);
        let mut args = SyscallArgs {
            arg1: self.0,
            ..Default::default()
        };
        let res = unsafe { syscall(index, &mut args) };
        match res.get(SyscallReturn::SYSCALL_TYPE) {
            SyscallReturnType::Error => {
                let code = res.get(SyscallReturn::SYSCALL_LEN);
                Err(abi::Error::from(code as u8))
            }
            SyscallReturnType::Copy => Ok(CapRef(res.get(SyscallReturn::SYSCALL_PTR) as usize)),
            _ => Err(abi::Error::ReturnTypeMismatch),
        }
    }
}

/// Receives requests from other threads
///
/// This function will block until until another thread sends a request to
/// the current thread
pub fn recv<T: ?Sized, R: Sized>(mask: u32, r: &mut T) -> Result<RecvResp<R>, Error> {
    let size = core::mem::size_of_val(r);
    let (ptr, _) = (r as *mut T).to_raw_parts();
    let index = SyscallIndex::new()
        .with(SyscallIndex::SYSCALL_ARG_TYPE, SyscallDataType::Copy)
        .with(SyscallIndex::SYSCALL_FN, SyscallFn::Recv);
    let mut resp: MaybeUninit<abi::RecvResp> = MaybeUninit::uninit();
    let mut args = SyscallArgs {
        arg1: ptr.addr(),
        arg2: size,
        arg3: mask as usize,
        arg4: resp.as_mut_ptr().addr(),
        ..Default::default()
    };
    let res = unsafe { syscall(index, &mut args) };
    match res.get(SyscallReturn::SYSCALL_TYPE) {
        SyscallReturnType::Error => {
            let code = res.get(SyscallReturn::SYSCALL_LEN);
            Err(abi::Error::from(code as u8))
        }
        _ => {
            let resp = unsafe { resp.assume_init() };
            Ok(RecvResp {
                cap: resp.cap,
                body: match resp.inner {
                    abi::RecvRespInner::Copy(len) => RecvRespBody::Copy(len),
                    abi::RecvRespInner::Page { addr, len } => {
                        if len != mem::size_of::<R>() {
                            return Err(Error::ReturnTypeMismatch);
                        }
                        RecvRespBody::Page(PageRefMut(unsafe { core::mem::transmute(addr) }))
                    }
                },
            })
        }
    }
}

#[derive(Format)]
pub struct RecvResp<T: ?Sized + 'static> {
    pub cap: Option<CapRef>,
    pub body: RecvRespBody<T>,
}

#[derive(Format)]
pub enum RecvRespBody<T: ?Sized + 'static> {
    Copy(usize),
    Page(PageRefMut<'static, T>),
}

/// Retrieves the tasks current capabilities
pub fn caps() -> Result<CapList, Error> {
    const ELEM: MaybeUninit<CapListEntry> = MaybeUninit::uninit();

    let index = SyscallIndex::new()
        .with(SyscallIndex::SYSCALL_ARG_TYPE, SyscallDataType::Short)
        .with(SyscallIndex::SYSCALL_FN, SyscallFn::Caps);
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

pub fn panik(buf: &mut [u8]) -> ! {
    unsafe {
        syscall(
            SyscallIndex::new()
                .with(SyscallIndex::SYSCALL_FN, SyscallFn::Panik)
                .with(SyscallIndex::SYSCALL_ARG_TYPE, SyscallDataType::Copy),
            &mut SyscallArgs {
                arg1: buf.as_mut_ptr().addr(),
                arg2: buf.len(),
                ..Default::default()
            },
        )
    };
    loop {} // will never be called sicne we paniked
}

pub struct CapList {
    buf: [MaybeUninit<abi::CapListEntry>; 10],
    len: usize,
}

impl Deref for CapList {
    type Target = [abi::CapListEntry];

    fn deref(&self) -> &Self::Target {
        // Safety:
        // The core invarient here is that when you create `CapList`
        // you ensure that 0..len items are initialized
        unsafe { core::mem::transmute(&self.buf[0..self.len]) }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut buf = LenWrite::default();
    let _ = write!(&mut buf, "{}", info); // our impl is infaillible
    panik(buf.buf())
}

struct LenWrite {
    buf: [u8; 512],
    pos: usize,
}

impl LenWrite {
    fn buf(&mut self) -> &mut [u8] {
        &mut self.buf[0..self.pos]
    }
}

impl Default for LenWrite {
    fn default() -> Self {
        Self {
            buf: [0; 512],
            pos: 0,
        }
    }
}

impl Write for LenWrite {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let start = self.pos;
        let (end, overflow) = self.pos.overflowing_add(bytes.len());
        if end >= 512 || overflow {
            return Ok(());
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.buf.get_unchecked_mut(start..end).as_mut_ptr(),
                bytes.len(),
            );
        }
        self.pos = end;
        Ok(())
    }
}

#[derive(defmt::Format)]
#[repr(C, align(32))]
pub struct Page<T: ?Sized>(pub T);

impl<T> Deref for Page<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Page<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Format)]
pub struct PageRefMut<'a, T: ?Sized + 'a>(&'a mut T);
impl<T> Deref for PageRefMut<'static, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for PageRefMut<'static, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub trait Aligned: Deref + DerefMut {}

impl<T> Aligned for Page<T> {}
impl<T> Aligned for PageRefMut<'static, T> {}
