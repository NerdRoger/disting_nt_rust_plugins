extern crate std;
use super::*;
use core::ptr;
use distingnt_sys::{
    NtAlgorithm, NtAlgorithmMemoryPtrs, NtAlgorithmRequirements, NtFactory, NtGlobals,
};
use std::alloc::{alloc_zeroed, dealloc, Layout};
type Bridge = distingnt_api::api::plugin::Registration<RustSynth, 0>;
unsafe fn factory() -> &'static NtFactory {
    &*(pluginEntry(SELECTOR_FACTORY_INFO, 0) as *const NtFactory)
}
struct Instance {
    base: *mut NtAlgorithm,
    sram: *mut u8,
    dtc: *mut u8,
    sram_layout: Layout,
    dtc_layout: Layout,
    values: [i16; 4],
}
impl Instance {
    fn new() -> Self {
        let mut req = NtAlgorithmRequirements {
            num_parameters: 0,
            sram: 0,
            dram: 0,
            dtc: 0,
            itc: 0,
        };
        unsafe {
            (factory().calculate_requirements.unwrap())(&mut req, ptr::null());
        }
        assert_eq!(req.dtc as usize, core::mem::size_of::<SynthState>());
        let sram_layout =
            Layout::from_size_align(req.sram as usize, Bridge::alignment(&[]).unwrap()).unwrap();
        let dtc_layout = Layout::new::<SynthState>();
        let sram = unsafe { alloc_zeroed(sram_layout) };
        let dtc = unsafe { alloc_zeroed(dtc_layout) };
        assert!(!sram.is_null() && !dtc.is_null());
        let memory = NtAlgorithmMemoryPtrs {
            sram,
            dtc,
            dram: ptr::null_mut(),
            itc: ptr::null_mut(),
        };
        let base = unsafe { (factory().construct.unwrap())(&memory, &req, ptr::null()) };
        assert!(!base.is_null());
        Self {
            base,
            sram,
            dtc,
            sram_layout,
            dtc_layout,
            values: [64, 1, 1, 0],
        }
    }
    fn step(&mut self, samples: &mut [f32]) {
        unsafe {
            (*self.base).v = self.values.as_ptr();
            (factory().step.unwrap())(self.base, samples.as_mut_ptr(), 1);
        }
    }
    fn midi(&mut self, status: u8, note: u8, velocity: u8) {
        unsafe {
            (*self.base).v = self.values.as_ptr();
            (factory().midi_message.unwrap())(self.base, status, note, velocity);
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.sram, self.sram_layout);
            dealloc(self.dtc, self.dtc_layout);
        }
    }
}
#[test]
fn routes_audio_to_bus_64_and_keeps_dsp_state_in_dtc() {
    let mut instance = Instance::new();
    instance.midi(0x90, 69, 100);
    let mut samples = [0.0; LAST_BUS * 4];
    instance.step(&mut samples);
    assert!(samples[..(LAST_BUS - 1) * 4].iter().all(|&v| v == 0.0));
    assert!(samples[(LAST_BUS - 1) * 4..].iter().all(|&v| v > 0.0));
    assert!(unsafe { (*instance.dtc.cast::<SynthState>()).gate });
    assert_eq!(pluginEntry(SELECTOR_VERSION, 0), API_VERSION_14);
}
#[test]
fn midi_filter_note_off_and_output_modes_preserve_behavior() {
    let mut instance = Instance::new();
    instance.midi(0x91, 69, 100);
    assert!(!unsafe { (*instance.dtc.cast::<SynthState>()).gate });
    instance.midi(0x90, 69, 100);
    assert!(unsafe { (*instance.dtc.cast::<SynthState>()).gate });
    instance.values[1] = 0;
    let mut samples = [1.0; LAST_BUS * 4];
    instance.step(&mut samples);
    assert!(samples[(LAST_BUS - 1) * 4..].iter().all(|&v| v > 1.0));
    instance.midi(0x90, 69, 0);
    assert!(!unsafe { (*instance.dtc.cast::<SynthState>()).gate });
}
#[no_mangle]
unsafe extern "C" fn rust_nt_globals() -> *const NtGlobals {
    static GLOBALS: NtGlobals = NtGlobals {
        sample_rate: 48000,
        max_frames_per_step: 128,
        work_buffer: ptr::null_mut(),
        work_buffer_size_bytes: 0,
        stream_size_bytes: 0,
        stream_buffer_size_bytes: 0,
    };
    &GLOBALS
}
#[no_mangle]
unsafe extern "C" fn exp2f(value: f32) -> f32 {
    (value as f64).exp2() as f32
}
