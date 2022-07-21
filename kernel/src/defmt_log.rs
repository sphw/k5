use defmt::global_logger;

#[global_logger]
struct KernelLogger;

static mut ENCODER: defmt::Encoder = ::defmt::Encoder::new();

::defmt::timestamp!("{=u32:us}", 0);

unsafe impl defmt::Logger for KernelLogger {
    fn acquire() {}

    unsafe fn flush() {}

    unsafe fn release() {}

    unsafe fn write(bytes: &[u8]) {
        ENCODER.write(bytes, |b| log(0, b));
    }
}

pub(crate) fn log(id: u8, log_buf: &[u8]) {
    let mut buf = [0u8; 257];
    buf[0] = id;
    buf[1] = log_buf.len() as u8;
    // NOTE: this assumes that the internal task index is the same as codegen task index, which is true for embedded,
    // but for systems with dynamic tasks is not true.
    buf[2..log_buf.len() + 2].clone_from_slice(log_buf);
    crate::arch::log(&buf[..log_buf.len() + 2]);
}
