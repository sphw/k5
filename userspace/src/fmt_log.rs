use core::fmt::Write;
pub fn _print(args: core::fmt::Arguments) {
    let mut buf = crate::LenWrite::default();
    buf.write_fmt(args).ok();
    buf.write_str("\r\n");
    crate::log(buf.buf());
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::_print(core::format_args!($($arg)*));
    }
}
#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {
        $crate::_print(core::format_args!($($arg)*));
        //$crate::log("\r\n".as_bytes()).ok();
    }
}
