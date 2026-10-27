//! Scala scale reading. Requests and output buffers must persist until callback.
use core::ffi::{c_char, c_void};
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtSclRatio {
    pub numerator_value: i32,
    pub denominator_value: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union NtSclNote {
    pub octaves: f64,
    pub ratio: NtSclRatio,
}
impl NtSclNote {
    /// Mirrors C++'s representation test; reading either union representation is
    /// valid for initialized data because both fields accept all bit patterns.
    #[inline]
    pub fn is_ratio(&self) -> bool {
        unsafe { self.ratio.denominator_value < 0 }
    }
    #[inline]
    pub fn numerator(&self) -> u32 {
        unsafe { self.ratio.numerator_value as u32 }
    }
    #[inline]
    pub fn denominator(&self) -> u32 {
        unsafe { self.ratio.denominator_value.wrapping_neg() as u32 }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtSclInfo {
    pub name: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtSclRequest {
    pub index: u32,
    pub notes: *mut NtSclNote,
    pub max_notes: u32,
    pub name_buffer: *mut c_char,
    pub name_buffer_size: usize,
    pub description_buffer: *mut c_char,
    pub description_buffer_size: usize,
    pub callback: Option<unsafe extern "C" fn(*mut c_void)>,
    pub callback_data: *mut c_void,
    pub error: bool,
    pub num_notes: u32,
}
unsafe extern "C" {
    pub fn NT_getNumScl() -> u32;
    pub fn NT_getSclInfo(index: u32, info: *mut NtSclInfo);
    pub fn NT_readScl(request: *mut NtSclRequest) -> bool;
}
