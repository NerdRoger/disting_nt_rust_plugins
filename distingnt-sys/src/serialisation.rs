//! C ABI forwarding for every public serialisation.h method. Host lifetimes and buffer contracts apply.
use crate::{NtJsonParse, NtJsonStream};
use core::ffi::c_char;
unsafe extern "C" {
    pub fn rust_nt_json_open_array(self_: *mut NtJsonStream);
    pub fn rust_nt_json_close_array(self_: *mut NtJsonStream);
    pub fn rust_nt_json_open_object(self_: *mut NtJsonStream);
    pub fn rust_nt_json_close_object(self_: *mut NtJsonStream);
    pub fn rust_nt_json_add_null(self_: *mut NtJsonStream);
    pub fn rust_nt_json_add_member_name(self_: *mut NtJsonStream, value: *const c_char);
    pub fn rust_nt_json_add_boolean(self_: *mut NtJsonStream, value: bool);
    pub fn rust_nt_json_add_number_i32(self_: *mut NtJsonStream, value: i32);
    pub fn rust_nt_json_add_number_f32(self_: *mut NtJsonStream, value: f32);
    pub fn rust_nt_json_add_string(self_: *mut NtJsonStream, value: *const c_char);
    pub fn rust_nt_json_add_four_cc(self_: *mut NtJsonStream, value: u32);
    pub fn rust_nt_json_number_of_object_members(self_: *mut NtJsonParse, value: *mut i32) -> bool;
    pub fn rust_nt_json_number_of_array_elements(self_: *mut NtJsonParse, value: *mut i32) -> bool;
    pub fn rust_nt_json_boolean(self_: *mut NtJsonParse, value: *mut bool) -> bool;
    pub fn rust_nt_json_number_i32(self_: *mut NtJsonParse, value: *mut i32) -> bool;
    pub fn rust_nt_json_number_f32(self_: *mut NtJsonParse, value: *mut f32) -> bool;
    pub fn rust_nt_json_string(self_: *mut NtJsonParse, value: *mut *const c_char) -> bool;
    pub fn rust_nt_json_match_name(self_: *mut NtJsonParse, name: *const c_char) -> bool;
    pub fn rust_nt_json_skip_member(self_: *mut NtJsonParse) -> bool;
    pub fn rust_nt_json_parse_null(self_: *mut NtJsonParse) -> bool;
}
