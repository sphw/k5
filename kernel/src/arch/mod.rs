#[cfg(feature = "cortex_m")]
pub mod cortex_m;
#[cfg(feature = "std")]
pub mod dummy;

#[cfg(feature = "cortex_m")]
pub use self::cortex_m::*;

#[cfg(feature = "std")]
pub use dummy::*;
