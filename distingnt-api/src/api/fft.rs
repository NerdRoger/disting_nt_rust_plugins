//! Firmware real FFTs over borrowed preallocated storage. No allocation/copying.
use crate::raw::dsp;
use core::marker::PhantomData;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidSize,
    StorageTooSmall,
    BufferSize,
    Misaligned,
}

pub const fn object_size(log2: u32) -> Option<usize> {
    dsp::fft_object_size(log2)
}

/// An exclusive handle; its internal firmware work buffer is not immutable.
/// It borrows the storage for its entire lifetime and cannot be cloned/copied.
pub struct Fft<'a> {
    storage: &'a mut [u32],
    samples: usize,
    not_send_sync: PhantomData<*mut ()>,
}
impl<'a> Fft<'a> {
    /// Construct in borrowed storage. No memory is retained after this handle
    /// is dropped. Check size and alignment before entering firmware.
    pub fn construct(storage: &'a mut [u32], log2: u32) -> Result<Self, Error> {
        let bytes = object_size(log2).ok_or(Error::InvalidSize)?;
        if storage.len() < bytes.div_ceil(4) {
            return Err(Error::StorageTooSmall);
        }
        if storage.as_ptr() as usize % 8 != 0 {
            return Err(Error::Misaligned);
        }
        // The header's object-size contract and native example admit caller
        // storage. Eight-byte alignment covers the ARM object's scalar fields.
        unsafe {
            dsp::NT_constructFFT(storage.as_mut_ptr().cast(), log2);
        }
        Ok(Self {
            storage,
            samples: 1usize << log2,
            not_send_sync: PhantomData,
        })
    }

    pub fn samples(&self) -> usize {
        self.samples
    }
    /// N floats: real bins 0..=N/2, imaginary bins 1..N/2 at N/2+k.
    pub fn forward(&mut self, time: &[f32], frequency: &mut [f32]) -> Result<(), Error> {
        if time.len() != self.samples || frequency.len() != self.samples {
            return Err(Error::BufferSize);
        }
        unsafe {
            dsp::NT_FFT(
                self.storage.as_ptr().cast(),
                frequency.as_mut_ptr(),
                time.as_ptr(),
            );
        }
        Ok(())
    }
    /// Firmware inverse is unnormalized: inverse(forward(x)) is N*x.
    pub fn inverse_unscaled(&mut self, frequency: &[f32], time: &mut [f32]) -> Result<(), Error> {
        if time.len() != self.samples || frequency.len() != self.samples {
            return Err(Error::BufferSize);
        }
        unsafe {
            dsp::NT_IFFT(
                self.storage.as_ptr().cast(),
                frequency.as_ptr(),
                time.as_mut_ptr(),
            );
        }
        Ok(())
    }
    /// Inverse followed by an explicit 1/N normalization pass.
    pub fn inverse(&mut self, frequency: &[f32], time: &mut [f32]) -> Result<(), Error> {
        self.inverse_unscaled(frequency, time)?;
        let scale = 1.0 / self.samples as f32;
        for sample in time {
            *sample *= scale;
        }
        Ok(())
    }
}

/// Movable, in-place initialized FFT storage for firmware-owned plugin state.
/// Private address tracking rebuilds the opaque plan after a move or Copy;
/// no copied internal firmware pointer is ever used at its previous address.
///
/// ```
/// #![forbid(unsafe_code)]
/// use distingnt_api::api::{fft::FftPlan, plugin::RegionInitializer};
/// #[derive(Clone,Copy)]
/// struct State { fft:FftPlan<642> }
/// fn initialize(destination:&mut RegionInitializer<'_,State>) {
///     distingnt_api::initialize_region!(destination, State {}, arrays {}, regions {
///         fft: |field| { FftPlan::initialize(field,8); }
///     });
/// }
/// fn process(state:&mut State, input:&[f32], spectrum:&mut[f32]) {
///     let _ = state.fft.forward(input,spectrum);
/// }
/// ```
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct FftPlan<const WORDS: usize> {
    storage: [u32; WORDS],
    log2: u32,
    initialized_at: usize,
}
impl<const WORDS: usize> FftPlan<WORDS> {
    /// Initialize directly in the firmware allocation; no temporary plan array.
    pub fn initialize(
        destination: &mut super::plugin::RegionInitializer<'_, Self>,
        log2: u32,
    ) -> Option<()> {
        let bytes = object_size(log2)?;
        if WORDS < bytes.div_ceil(4) {
            return None;
        }
        let pointer = destination.__destination();
        // RegionInitializer admits exclusive aligned storage for this type.
        unsafe {
            core::ptr::addr_of_mut!((*pointer).storage)
                .cast::<u32>()
                .write_bytes(0, WORDS);
            core::ptr::addr_of_mut!((*pointer).log2).write(log2);
            core::ptr::addr_of_mut!((*pointer).initialized_at).write(0);
            destination.__finish();
        }
        Some(())
    }
    fn handle(&mut self) -> Fft<'_> {
        let address = self.storage.as_ptr() as usize;
        if self.initialized_at != address {
            unsafe {
                dsp::NT_constructFFT(self.storage.as_mut_ptr().cast(), self.log2);
            }
            self.initialized_at = address;
        }
        Fft {
            storage: &mut self.storage,
            samples: 1usize << self.log2,
            not_send_sync: PhantomData,
        }
    }
    pub fn samples(&self) -> usize {
        1usize << self.log2
    }
    pub fn forward(&mut self, time: &[f32], frequency: &mut [f32]) -> Result<(), Error> {
        if time.len() != self.samples() || frequency.len() != self.samples() {
            return Err(Error::BufferSize);
        }
        self.handle().forward(time, frequency)
    }
    pub fn inverse_unscaled(&mut self, frequency: &[f32], time: &mut [f32]) -> Result<(), Error> {
        if time.len() != self.samples() || frequency.len() != self.samples() {
            return Err(Error::BufferSize);
        }
        self.handle().inverse_unscaled(frequency, time)
    }
    pub fn inverse(&mut self, frequency: &[f32], time: &mut [f32]) -> Result<(), Error> {
        if time.len() != self.samples() || frequency.len() != self.samples() {
            return Err(Error::BufferSize);
        }
        self.handle().inverse(frequency, time)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::cell::Cell;
    #[repr(align(8))]
    struct Aligned([u32; 22]);
    std::thread_local! { static CALLS: Cell<u32> = const { Cell::new(0) };
    static CONSTRUCTIONS: Cell<u32> = const { Cell::new(0) }; }

    #[no_mangle]
    unsafe extern "C" fn NT_constructFFT(plan: *const u8, log2: u32) {
        CONSTRUCTIONS.with(|c| c.set(c.get() + 1));
        unsafe {
            plan.cast_mut().cast::<u32>().write(1 << log2);
        }
    }
    // Native reference DFT stubs test adaptation, layout and normalization;
    // they do not emulate firmware performance or establish its transform sign.
    #[no_mangle]
    unsafe extern "C" fn NT_FFT(plan: *const u8, out: *mut f32, input: *const f32) {
        CALLS.with(|c| c.set(c.get() + 1));
        let n = unsafe { *plan.cast::<u32>() } as usize;
        let input = unsafe { core::slice::from_raw_parts(input, n) };
        let out = unsafe { core::slice::from_raw_parts_mut(out, n) };
        out.fill(0.0);
        for k in 0..=n / 2 {
            for (i, &x) in input.iter().enumerate() {
                let phase = 2.0 * std::f64::consts::PI * k as f64 * i as f64 / n as f64;
                out[k] += x * phase.cos() as f32;
                if k > 0 && k < n / 2 {
                    out[n / 2 + k] -= x * phase.sin() as f32;
                }
            }
        }
    }
    #[no_mangle]
    unsafe extern "C" fn NT_IFFT(plan: *const u8, input: *const f32, out: *mut f32) {
        CALLS.with(|c| c.set(c.get() + 1));
        let n = unsafe { *plan.cast::<u32>() } as usize;
        let input = unsafe { core::slice::from_raw_parts(input, n) };
        let out = unsafe { core::slice::from_raw_parts_mut(out, n) };
        for (i, x) in out.iter_mut().enumerate() {
            *x = input[0]
                + if i % 2 == 0 {
                    input[n / 2]
                } else {
                    -input[n / 2]
                };
            for k in 1..n / 2 {
                let phase = 2.0 * std::f64::consts::PI * k as f64 * i as f64 / n as f64;
                *x += 2.0 * (input[k] * phase.cos() as f32 - input[n / 2 + k] * phase.sin() as f32);
            }
        }
    }
    #[test]
    fn real_transform_roundtrip_and_unscaled_inverse_use_supplied_storage() {
        let mut aligned = Aligned([0u32; 22]);
        let storage = &mut aligned.0;
        let mut fft = Fft::construct(storage, 3).unwrap();
        let input = [0.0, 0.5, 1.0, -0.5, 0.25, -1.0, 0.2, 0.0];
        let mut spectrum = [0.0; 8];
        let mut output = [0.0; 8];
        fft.forward(&input, &mut spectrum).unwrap();
        fft.inverse_unscaled(&spectrum, &mut output).unwrap();
        for (a, b) in output.iter().zip(input) {
            assert!((*a - 8.0 * b).abs() < 0.00001);
        }
        fft.inverse(&spectrum, &mut output).unwrap();
        for (a, b) in output.iter().zip(input) {
            assert!((*a - b).abs() < 0.00001);
        }
        assert_eq!(fft.samples(), 8);
    }
    #[test]
    fn storage_and_slice_validation_prevent_invalid_host_calls() {
        assert!(matches!(
            Fft::construct(&mut [0u32; 1], 3),
            Err(Error::StorageTooSmall)
        ));
        assert!(matches!(
            Fft::construct(&mut [], 0),
            Err(Error::InvalidSize)
        ));
        let mut aligned = Aligned([0u32; 22]);
        let storage = &mut aligned.0;
        let mut fft = Fft::construct(storage, 3).unwrap();
        CALLS.with(|c| c.set(0));
        assert_eq!(
            fft.forward(&[0.0; 7], &mut [0.0; 8]),
            Err(Error::BufferSize)
        );
        assert_eq!(
            fft.inverse(&[0.0; 8], &mut [0.0; 9]),
            Err(Error::BufferSize)
        );
        CALLS.with(|c| assert_eq!(c.get(), 0));
    }
    #[test]
    fn owned_plan_initializes_in_place_reuses_and_rebuilds_after_copy() {
        use super::super::plugin::RegionInitializer;
        use core::mem::MaybeUninit;
        let mut storage = MaybeUninit::<FftPlan<22>>::uninit();
        let mut init = unsafe { RegionInitializer::new(storage.as_mut_ptr()) };
        assert!(FftPlan::initialize(&mut init, 3).is_some());

        let first = unsafe { &mut *storage.as_mut_ptr() };
        let input = [1.0; 8];
        let mut spectrum = [0.0; 8];
        CONSTRUCTIONS.with(|c| c.set(0));
        first.forward(&input, &mut spectrum).unwrap();
        first.forward(&input, &mut spectrum).unwrap();
        CONSTRUCTIONS.with(|c| assert_eq!(c.get(), 1));
        let mut second = MaybeUninit::<FftPlan<22>>::uninit();
        unsafe {
            second.as_mut_ptr().write(*first);
        }
        let second = unsafe { &mut *second.as_mut_ptr() };
        assert_ne!(first.storage.as_ptr(), second.storage.as_ptr());
        second.forward(&input, &mut spectrum).unwrap();
        CONSTRUCTIONS.with(|c| assert_eq!(c.get(), 2));
        first.forward(&input, &mut spectrum).unwrap();
        CONSTRUCTIONS.with(|c| assert_eq!(c.get(), 2));
    }
}
