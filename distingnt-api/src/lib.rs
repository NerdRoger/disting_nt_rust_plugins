//! Safe plugin framework and Rust adapters. Raw bindings live in distingnt-sys.
//!
//! Raw modules and host functions are deliberately not part of this facade:
//! ```compile_fail
//! use distingnt_api::raw;
//! ```
//! ```compile_fail
//! use distingnt_api::NT_algorithmIndex;
//! ```
#![no_std]

// Internal imports are deliberately private; no raw ABI functions or pointer
// types are re-exported by this crate.
pub(crate) use distingnt_sys as raw;
pub(crate) use distingnt_sys::serialisation;
pub mod api;

// Shared scalar constants and pointer-free event values are safe facade values.
pub use distingnt_sys::multichar;
pub use distingnt_sys::nt_dram_section;
pub use distingnt_sys::{
    NtFloat3, NtUiData, API_VERSION_1, API_VERSION_10, API_VERSION_11, API_VERSION_12,
    API_VERSION_13, API_VERSION_14, API_VERSION_2, API_VERSION_3, API_VERSION_4, API_VERSION_5,
    API_VERSION_6, API_VERSION_7, API_VERSION_8, API_VERSION_9, API_VERSION_CURRENT, BUTTON_1,
    BUTTON_2, BUTTON_3, BUTTON_4, DESTINATION_BREAKOUT, DESTINATION_INTERNAL,
    DESTINATION_SELECT_BUS, DESTINATION_USB, ENCODER_BUTTON_L, ENCODER_BUTTON_R, ENCODER_L,
    ENCODER_R, LAST_BUS, MINIMUM_STEPS_IN_WORK_BUFFER, NUM_AUX_BUSSES, NUM_INPUT_BUSSES,
    NUM_OUTPUT_BUSSES, PARAMETER_STRING_SIZE, PARAMETER_UI_PREFIX_SIZE, POT_BUTTON_C, POT_BUTTON_L,
    POT_BUTTON_R, POT_C, POT_L, POT_R, SCALING_10, SCALING_100, SCALING_1000, SCALING_NONE,
    SCREEN_BYTES, SELECTOR_FACTORY_INFO, SELECTOR_NUM_FACTORIES, SELECTOR_VERSION, SHAPE_BOX,
    SHAPE_CIRCLE, SHAPE_LINE, SHAPE_POINT, SHAPE_RECTANGLE, TAG_DELAY, TAG_EFFECT, TAG_FILTER_EQ,
    TAG_INSTRUMENT, TAG_REVERB, TAG_UTILITY, TEXT_CENTRE, TEXT_LARGE, TEXT_LEFT, TEXT_NORMAL,
    TEXT_RIGHT, TEXT_TINY, TYPE_BOOLEAN, TYPE_GENERIC, TYPE_SECONDS, UNIT_AUDIO_INPUT,
    UNIT_AUDIO_OUTPUT, UNIT_BPM, UNIT_CENTS, UNIT_CONFIRM, UNIT_CV_INPUT, UNIT_CV_OUTPUT, UNIT_DB,
    UNIT_DB_MIN_INF, UNIT_DEGREES, UNIT_ENUM, UNIT_FRAMES, UNIT_HAS_STRINGS, UNIT_HZ,
    UNIT_MIDI_NOTE, UNIT_MILLIVOLTS, UNIT_MS, UNIT_NONE, UNIT_OUTPUT_MODE, UNIT_PERCENT,
    UNIT_SECONDS, UNIT_SEMITONES, UNIT_STRING, UNIT_VOLTS,
};
