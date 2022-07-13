#![no_std]
#![feature(naked_functions)]
#![feature(strict_provenance)]
#![feature(ptr_metadata)]
#![feature(asm_sym)]
mod cortex_m;
pub use cortex_m::*;

mod defmt;
