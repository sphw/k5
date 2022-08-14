#[cfg(feature = "cortex_m")]
pub mod cortex_m;
#[cfg(feature = "std")]
pub mod dummy;
#[cfg(feature = "rv64")]
pub mod rv64;

#[cfg(feature = "cortex_m")]
pub use self::cortex_m::*;

#[cfg(feature = "rv64")]
pub use self::rv64::*;

#[cfg(feature = "std")]
pub use dummy::*;
