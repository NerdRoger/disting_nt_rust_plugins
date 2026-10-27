//! Host math routines with scalar Rust interfaces.
#[inline]
pub fn exp2(value: f32) -> f32 {
    // SAFETY: exp2f accepts every f32 and has no pointer/lifetime preconditions.
    unsafe { crate::raw::exp2f(value) }
}
