//! Host formatting into caller-owned bounded buffers.
use crate::raw::{NT_floatToString, NT_intToString};
use core::ffi::CStr;

/// Stack-owned, terminated host text. The initialized length is kept so drawing
/// it does not need to repeat a generic C-string search.
pub struct TextBuffer<const N: usize> {
    bytes: [u8; N],
    length: usize,
}

impl<const N: usize> Default for TextBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> TextBuffer<N> {
    pub const fn new() -> Self {
        assert!(N > 0, "text buffers need a terminator");
        Self {
            bytes: [0; N],
            length: 0,
        }
    }

    /// On a formatting failure, retain the prefix. An oversized prefix produces
    /// empty text. Interior NULs terminate the result, including when the host
    /// reports an in-bounds length beyond the actual end of its output.
    pub fn format_float(&mut self, prefix: &[u8], value: f32, decimals: u8) -> bool {
        self.length = 0;
        self.bytes[0] = 0;
        if prefix.len() >= N {
            return false;
        }
        self.bytes[..prefix.len()].copy_from_slice(prefix);
        let reported = format_float(&mut self.bytes[prefix.len()..], value, decimals);
        let end = prefix.len() + reported.unwrap_or(0);
        // A small bounded scan also handles malformed host lengths. Do this once
        // when writing; consumers receive an already validated CStr.
        while self.length < end && self.bytes.get(self.length).is_some_and(|&byte| byte != 0) {
            self.length += 1;
        }
        if let Some(terminator) = self.bytes.get_mut(self.length) {
            *terminator = 0;
        }
        reported.is_some()
    }

    #[inline]
    pub fn as_c_str(&self) -> &CStr {
        // SAFETY: new and format_float keep length < N, put NUL at length,
        // and establish that no earlier byte is NUL. Storage is private.
        unsafe { CStr::from_bytes_with_nul_unchecked(self.bytes.get_unchecked(..=self.length)) }
    }
}

#[inline]
fn finish(buffer: &mut [u8], reported: i32) -> Option<usize> {
    if let Ok(length) = usize::try_from(reported) {
        if let Some(terminator) = buffer.get_mut(length) {
            *terminator = 0;
            return Some(length);
        }
    }
    if let Some(first) = buffer.first_mut() {
        *first = 0;
    }
    None
}

#[inline]
pub fn format_integer(buffer: &mut [u8], value: i32) -> Option<usize> {
    // Sign, ten decimal digits, terminator, even for i32::MIN.
    if buffer.len() < 12 {
        if let Some(first) = buffer.first_mut() {
            *first = 0;
        }
        return None;
    }
    // SAFETY: the host consumes this exclusive buffer synchronously; any i32 fits.
    let length = unsafe { NT_intToString(buffer.as_mut_ptr().cast(), value) };
    finish(buffer, length)
}

/// The same bounded formatter for C ABI callback output buffers.
#[inline]
pub fn format_integer_chars(buffer: &mut [core::ffi::c_char], value: i32) -> Option<usize> {
    // SAFETY: c_char and u8 are one-byte, alignment-one types with every bit
    // pattern valid. This exclusive reborrow ends before the original is reused.
    let bytes =
        unsafe { core::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), buffer.len()) };
    format_integer(bytes, value)
}

#[inline]
pub fn format_float(buffer: &mut [u8], value: f32, decimals: u8) -> Option<usize> {
    // At most 39 integer digits, sign, decimal point, requested fraction and NUL.
    // Nonfinite strings are shorter. The host formatter must obey its contract.
    if buffer.len() < 42 + decimals as usize {
        if let Some(first) = buffer.first_mut() {
            *first = 0;
        }
        return None;
    }
    // SAFETY: the conservative capacity check covers every formatted f32.
    let length = unsafe { NT_floatToString(buffer.as_mut_ptr().cast(), value, decimals as i32) };
    finish(buffer, length)
}
