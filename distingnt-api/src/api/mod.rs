//! Idiomatic host plumbing above the private distingnt-sys dependency.
//! No allocation or dynamic dispatch.
//!
//! The framework admits callback pointers internally. Plugin callbacks receive
//! bounded buffers, copied values, and borrowed CStrs through the safe facade.
//! They must stay within the originating callback; reentrant setters must never
//! run while plugin state or host parameter storage is borrowed incompatibly.
#![deny(unsafe_op_in_unsafe_fn)]

pub mod audio;
pub mod drawing;
pub mod json;
pub mod math;
pub mod metadata;
pub mod parameters;
pub mod plugin;
pub mod state_memory;
pub mod text;

#[cfg(test)]
mod tests;

/// The host's persistent sample rate, accessed through the supported C++ shim.
#[inline]
pub fn sample_rate() -> u32 {
    // SAFETY: plugin code runs in a host that supplies persistent readable globals.
    unsafe { (*crate::raw::rust_nt_globals()).sample_rate }
}
