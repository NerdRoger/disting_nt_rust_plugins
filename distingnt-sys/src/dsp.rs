//! Direct C ABI from distingnt/dsp.h. The const pointer is opaque storage:
//! construction and transforms mutate it internally. Do not share concurrently.
#[allow(non_camel_case_types)]
pub type NtFft = *const u8;

/// Checked equivalent of the upstream inline NT_FFT_objectSize formula.
/// Rejects shifts/arithmetic outside the header's defined signed-int domain.
/// This is a storage calculation, not a promise the firmware supports a size.
pub const fn fft_object_size(log2: u32) -> Option<usize> {
    if log2 < 1 || log2 > 27 {
        return None;
    }
    let samples = 1usize << log2;
    Some(8 + 10 * samples)
}

unsafe extern "C" {
    pub fn NT_constructFFT(fft: NtFft, fft_size_log2: u32);
    pub fn NT_FFT(fft: NtFft, frequency: *mut f32, time: *const f32);
    pub fn NT_IFFT(fft: NtFft, frequency: *const f32, time: *mut f32);
}
