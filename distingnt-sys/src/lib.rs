#![no_std]

pub mod raw;
pub use raw::*;
pub mod file;
pub mod microtuning;
pub mod serialisation;
pub mod slot;
pub mod wav;
pub use serialisation::*;
#[cfg(test)]
mod tests;

/// Place functions in Disting's DRAM code section. Prevents the annotated item
/// from being inlined back into an ITC caller. Can wrap functions or impl methods.
#[macro_export]
macro_rules! nt_dram_section {
    ($($item:item)*) => {$(
        #[inline(never)]
        #[unsafe(link_section = ".text.nt_dram")]
        $item
    )*};
}

#[macro_export]
macro_rules! nt_parameter_audio_input {
    ($n:expr,$m:expr,$d:expr) => {
        $crate::nt_parameter_io!($n, $m, $d, $crate::UNIT_AUDIO_INPUT)
    };
}
#[macro_export]
macro_rules! nt_parameter_cv_input {
    ($n:expr,$m:expr,$d:expr) => {
        $crate::nt_parameter_io!($n, $m, $d, $crate::UNIT_CV_INPUT)
    };
}
#[macro_export]
macro_rules! nt_parameter_audio_output {
    ($n:expr,$m:expr,$d:expr) => {
        $crate::nt_parameter_io!($n, $m, $d, $crate::UNIT_AUDIO_OUTPUT)
    };
}
#[macro_export]
macro_rules! nt_parameter_cv_output {
    ($n:expr,$m:expr,$d:expr) => {
        $crate::nt_parameter_io!($n, $m, $d, $crate::UNIT_CV_OUTPUT)
    };
}
/// Returns two definitions, corresponding to the two C macro initializers.
/// The name is a literal so the mode-name string can be constructed at compile time.
#[macro_export]
macro_rules! nt_parameter_audio_output_with_mode {
    ($n:literal,$m:expr,$d:expr) => {
        [
            $crate::nt_parameter_audio_output!(concat!($n, "\0").as_ptr().cast(), $m, $d),
            $crate::nt_parameter_output_mode!(concat!($n, " mode\0").as_ptr().cast()),
        ]
    };
}
#[macro_export]
macro_rules! nt_parameter_cv_output_with_mode {
    ($n:literal,$m:expr,$d:expr) => {
        [
            $crate::nt_parameter_cv_output!(concat!($n, "\0").as_ptr().cast(), $m, $d),
            $crate::nt_parameter_output_mode!(concat!($n, " mode\0").as_ptr().cast()),
        ]
    };
}
