//! Borrowed host JSON handles. Their constructors establish the FFI contract.
use crate::raw::{NtJsonParse, NtJsonStream};
use crate::serialisation::*;
use core::{ffi::CStr, marker::PhantomData};

/// Minimal JSON operations needed by reusable preset readers. Mockable in tests.
pub trait Reader {
    fn member_count(&mut self) -> Option<i32>;
    fn matches(&mut self, name: &CStr) -> bool;
    fn boolean(&mut self, value: &mut bool) -> bool;
    fn skip(&mut self) -> bool;
}
/// Mockable member writer; concrete JsonWriter also exposes other scalar methods.
pub trait Writer {
    fn boolean_member(&mut self, name: &CStr, value: bool);
}

pub struct JsonReader<'a> {
    raw: *mut NtJsonParse,
    _borrow: PhantomData<&'a mut NtJsonParse>,
}

impl JsonReader<'_> {
    /// # Safety
    /// The pointer must be a valid host parser, exclusively available for the
    /// adapter's lifetime. The adapter must not outlive the deserialise callback.
    #[inline]
    pub(crate) unsafe fn new(raw: *mut NtJsonParse) -> Self {
        Self {
            raw,
            _borrow: PhantomData,
        }
    }
}

impl Reader for JsonReader<'_> {
    // The constructor establishes parser validity/exclusivity. Each method
    // lends only call-local outputs or a CStr that the host consumes immediately.
    #[inline]
    fn member_count(&mut self) -> Option<i32> {
        let mut count = 0;
        unsafe { rust_nt_json_number_of_object_members(self.raw, &mut count) }.then_some(count)
    }
    #[inline]
    fn matches(&mut self, name: &CStr) -> bool {
        unsafe { rust_nt_json_match_name(self.raw, name.as_ptr()) }
    }
    #[inline]
    fn boolean(&mut self, value: &mut bool) -> bool {
        unsafe { rust_nt_json_boolean(self.raw, value) }
    }
    #[inline]
    fn skip(&mut self) -> bool {
        unsafe { rust_nt_json_skip_member(self.raw) }
    }
}

pub struct JsonWriter<'a> {
    raw: *mut NtJsonStream,
    _borrow: PhantomData<&'a mut NtJsonStream>,
}

impl JsonWriter<'_> {
    /// # Safety
    /// The pointer must be a valid host stream, exclusively available for the
    /// adapter's lifetime. The adapter must not outlive the serialise callback.
    #[inline]
    pub(crate) unsafe fn new(raw: *mut NtJsonStream) -> Self {
        Self {
            raw,
            _borrow: PhantomData,
        }
    }
}

impl Writer for JsonWriter<'_> {
    // The constructor establishes stream validity/exclusivity; the host consumes
    // the member name during this call and does not retain its pointer.
    #[inline]
    fn boolean_member(&mut self, name: &CStr, value: bool) {
        unsafe {
            rust_nt_json_add_member_name(self.raw, name.as_ptr());
            rust_nt_json_add_boolean(self.raw, value);
        }
    }
}
impl JsonWriter<'_> {
    #[inline]
    pub fn open_object(&mut self) {
        unsafe {
            rust_nt_json_open_object(self.raw);
        }
    }
    #[inline]
    pub fn close_object(&mut self) {
        unsafe {
            rust_nt_json_close_object(self.raw);
        }
    }
    #[inline]
    pub fn open_array(&mut self) {
        unsafe {
            rust_nt_json_open_array(self.raw);
        }
    }
    #[inline]
    pub fn close_array(&mut self) {
        unsafe {
            rust_nt_json_close_array(self.raw);
        }
    }
    #[inline]
    pub fn member(&mut self, name: &CStr) {
        unsafe {
            rust_nt_json_add_member_name(self.raw, name.as_ptr());
        }
    }
    #[inline]
    pub fn null(&mut self) {
        unsafe {
            rust_nt_json_add_null(self.raw);
        }
    }
    #[inline]
    pub fn boolean(&mut self, value: bool) {
        unsafe {
            rust_nt_json_add_boolean(self.raw, value);
        }
    }
    #[inline]
    pub fn integer(&mut self, value: i32) {
        unsafe {
            rust_nt_json_add_number_i32(self.raw, value);
        }
    }
    #[inline]
    pub fn float(&mut self, value: f32) {
        unsafe {
            rust_nt_json_add_number_f32(self.raw, value);
        }
    }
    #[inline]
    pub fn string(&mut self, value: &CStr) {
        unsafe {
            rust_nt_json_add_string(self.raw, value.as_ptr());
        }
    }
    #[inline]
    pub fn four_cc(&mut self, value: u32) {
        unsafe {
            rust_nt_json_add_four_cc(self.raw, value);
        }
    }
}
impl JsonReader<'_> {
    #[inline]
    pub fn array_count(&mut self) -> Option<i32> {
        let mut value = 0;
        unsafe { rust_nt_json_number_of_array_elements(self.raw, &mut value) }.then_some(value)
    }
    #[inline]
    pub fn integer(&mut self) -> Option<i32> {
        let mut value = 0;
        unsafe { rust_nt_json_number_i32(self.raw, &mut value) }.then_some(value)
    }
    #[inline]
    pub fn float(&mut self) -> Option<f32> {
        let mut value = 0.0;
        unsafe { rust_nt_json_number_f32(self.raw, &mut value) }.then_some(value)
    }
    #[inline]
    pub fn null(&mut self) -> bool {
        unsafe { rust_nt_json_parse_null(self.raw) }
    }
}
