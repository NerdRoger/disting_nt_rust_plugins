//! Static strings converted to the host's thin-pointer enum table.
use core::ffi::{c_char, CStr};

/// An immutable static C name stored as one pointer, without a redundant length.
/// Only validated static C strings can create names; no raw pointer is exposed.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Name(*const c_char);
// SAFETY: the pointer always targets an immutable static CStr.
unsafe impl Sync for Name {}
unsafe impl Send for Name {}

impl Name {
    pub const fn new(value: &'static CStr) -> Self {
        Self(value.as_ptr())
    }
    pub(super) const fn as_ptr(self) -> *const c_char {
        self.0
    }
}

pub struct EnumStrings<const N: usize>([*const c_char; N]);
// SAFETY: construction accepts only immutable static C strings; the table has
// no interior mutability and cannot be modified through its public interface.
unsafe impl<const N: usize> Sync for EnumStrings<N> {}

impl<const N: usize> EnumStrings<N> {
    pub const fn new(strings: [&'static CStr; N]) -> Self {
        let mut pointers = [core::ptr::null(); N];
        let mut index = 0;
        while index < N {
            pointers[index] = strings[index].as_ptr();
            index += 1;
        }
        Self(pointers)
    }
    #[cfg(test)]
    pub(crate) const fn as_ptr(&'static self) -> *const *const c_char {
        self.0.as_ptr()
    }
    pub const fn table(&'static self) -> EnumTable {
        EnumTable {
            raw: self.0.as_ptr(),
            len: N,
        }
    }
}

#[derive(Clone, Copy)]
pub struct EnumTable {
    raw: *const *const c_char,
    len: usize,
}
// SAFETY: only EnumStrings::table creates this immutable static view.
unsafe impl Sync for EnumTable {}

/// Validated, pointer-safe parameter definition. Constant definitions have the
/// same footprint as the raw ABI; validation is evaluated at compile time.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Definition(crate::raw::NtParameter);

impl Definition {
    /// Reuse validated numeric metadata with a different immutable static name.
    pub const fn with_name(mut self, name: Name) -> Self {
        self.0.name = name.as_ptr();
        self
    }

    #[inline]
    pub const fn new(
        name: &'static CStr,
        min: i16,
        max: i16,
        default: i16,
        unit: u8,
        scaling: u8,
        enum_strings: Option<EnumTable>,
    ) -> Option<Self> {
        use crate::raw::*;
        if min > default || default > max || scaling > SCALING_1000 {
            return None;
        }
        if !(unit <= UNIT_STRING || (unit >= UNIT_AUDIO_INPUT && unit <= UNIT_OUTPUT_MODE)) {
            return None;
        }
        if unit >= UNIT_AUDIO_INPUT && unit <= UNIT_CV_OUTPUT && (min < 0 || max > LAST_BUS as i16)
        {
            return None;
        }
        if unit == UNIT_OUTPUT_MODE && (min < 0 || max > 1) {
            return None;
        }
        if unit == UNIT_ENUM {
            match enum_strings {
                Some(table) if min >= 0 && (max as usize) < table.len => {}
                _ => return None,
            }
        }
        let enums = match enum_strings {
            Some(table) => table.raw,
            None => core::ptr::null(),
        };
        Some(Self(NtParameter {
            name: name.as_ptr(),
            min,
            max,
            default,
            unit,
            scaling,
            enum_strings: enums,
        }))
    }
    pub(super) const fn raw(self) -> crate::raw::NtParameter {
        self.0
    }
}
