extern crate std;
use super::*;
use std::ffi::CStr;

#[test]
fn io_macros_follow_v14_bus_limits_and_mode_names() {
    let input = nt_parameter_cv_input!(c"Input".as_ptr(), 0, 1);
    assert_eq!(input.max, LAST_BUS as i16);
    assert_eq!(input.unit, UNIT_CV_INPUT);
    let [output, mode] = nt_parameter_audio_output_with_mode!("Output", 0, 13);
    assert_eq!(output.max, 64);
    assert_eq!(mode.unit, UNIT_OUTPUT_MODE);
    assert_eq!(unsafe { CStr::from_ptr(mode.name) }, c"Output mode");
    let [output, mode] = nt_parameter_cv_output_with_mode!("CV", 0, 1);
    assert_eq!(output.unit, UNIT_CV_OUTPUT);
    assert_eq!(unsafe { CStr::from_ptr(mode.name) }, c"CV mode");
}

#[test]
fn scale_note_union_helpers_preserve_ratio_representation() {
    use microtuning::*;
    let ratio = NtSclNote {
        ratio: NtSclRatio {
            numerator_value: 3,
            denominator_value: -2,
        },
    };
    assert!(ratio.is_ratio());
    assert_eq!(ratio.numerator(), 3);
    assert_eq!(ratio.denominator(), 2);
    assert!(!NtSclNote { octaves: 1.0 }.is_ratio());
}

#[test]
fn string_buffer_callback_exposes_pointer_and_capacity() {
    unsafe extern "C" fn buffer(
        self_: *mut NtAlgorithm,
        _: i32,
        destination: *mut *mut core::ffi::c_char,
    ) -> i32 {
        *destination = self_.cast();
        64
    }
    let callback: StringParameterBuffer = buffer;
    let mut storage = [0u8; 64];
    let mut result = core::ptr::null_mut();
    assert_eq!(
        unsafe { callback(storage.as_mut_ptr().cast(), 0, &mut result) },
        64
    );
    assert_eq!(result, storage.as_mut_ptr().cast());
}
