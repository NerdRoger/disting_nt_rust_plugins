//! C ABI forwarding for every public slot.h method. Host lifetimes and buffer contracts apply.
use crate::{NtAlgorithm, NtParameter, NtSlot};
use core::ffi::c_char;
unsafe extern "C" {
    pub fn rust_nt_slot_name(self_: *const NtSlot) -> *const c_char;
    pub fn rust_nt_slot_guid(self_: *const NtSlot) -> u32;
    pub fn rust_nt_slot_plugin(self_: *const NtSlot) -> *mut NtAlgorithm;
    pub fn rust_nt_slot_num_parameters(self_: *const NtSlot) -> u32;
    pub fn rust_nt_slot_parameter_info(
        self_: *const NtSlot,
        info: *mut NtParameter,
        index: u32,
    ) -> bool;
    pub fn rust_nt_slot_parameter_value(self_: *const NtSlot, index: u32) -> i16;
    pub fn rust_nt_slot_parameter_preset_value(self_: *const NtSlot, index: u32) -> i16;
    pub fn rust_nt_slot_parameter_string(
        self_: *const NtSlot,
        index: u32,
        buffer: *mut c_char,
    ) -> u32;
}
