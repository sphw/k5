use defmt::global_logger;

#[global_logger]
struct KernelLogger;

static mut ENCODER: defmt::Encoder = defmt::Encoder::new();

defmt::timestamp!("{=u32:us}", 0);

unsafe impl defmt::Logger for KernelLogger {
    fn acquire() {}

    unsafe fn flush() {}

    unsafe fn release() {}

    unsafe fn write(bytes: &[u8]) {
        ENCODER.write(bytes, |b| {
            let _ = crate::log(b);
        });
    }
}
