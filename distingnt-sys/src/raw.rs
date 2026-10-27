//! Raw ABI bindings from `distingnt/api.h` (API v14).
//!
//! Names use idiomatic Rust spelling; each item has the exact C layout or
//! function signature from the header.

use core::ffi::c_char;

// Other upstream headers, also available through the legacy crate-root paths.
pub use crate::{file, microtuning, serialisation, slot, wav};

pub const API_VERSION_1: usize = 1;
pub const API_VERSION_2: usize = 2;
pub const API_VERSION_3: usize = 3;
pub const API_VERSION_4: usize = 4;
pub const API_VERSION_5: usize = 5;
pub const API_VERSION_6: usize = 6;
pub const API_VERSION_7: usize = 7;
pub const API_VERSION_8: usize = 8;
pub const API_VERSION_9: usize = 9;
pub const API_VERSION_10: usize = 10;
pub const API_VERSION_11: usize = 11;
pub const API_VERSION_12: usize = 12;
pub const API_VERSION_13: usize = 13;
pub const API_VERSION_14: usize = 14;
pub const API_VERSION_CURRENT: usize = API_VERSION_14;
pub const NUM_INPUT_BUSSES: usize = 12;
pub const NUM_OUTPUT_BUSSES: usize = 8;
pub const NUM_AUX_BUSSES: usize = 44;
pub const LAST_BUS: usize = NUM_INPUT_BUSSES + NUM_OUTPUT_BUSSES + NUM_AUX_BUSSES;
pub const MINIMUM_STEPS_IN_WORK_BUFFER: usize = 12;
pub const SELECTOR_VERSION: u8 = 0;
pub const SELECTOR_NUM_FACTORIES: u8 = 1;
pub const SELECTOR_FACTORY_INFO: u8 = 2;
pub const TEXT_TINY: u8 = 0;
pub const TEXT_NORMAL: u8 = 1;
pub const TEXT_LARGE: u8 = 2;
pub const TEXT_LEFT: u8 = 0;
pub const TEXT_CENTRE: u8 = 1;
pub const TEXT_RIGHT: u8 = 2;
pub const SHAPE_POINT: u8 = 0;
pub const SHAPE_LINE: u8 = 1;
pub const SHAPE_BOX: u8 = 2;
pub const SHAPE_RECTANGLE: u8 = 3;
pub const SHAPE_CIRCLE: u8 = 4;
pub const DESTINATION_BREAKOUT: u32 = 1 << 0;
pub const DESTINATION_SELECT_BUS: u32 = 1 << 1;
pub const DESTINATION_USB: u32 = 1 << 2;
pub const DESTINATION_INTERNAL: u32 = 1 << 3;

pub const UNIT_NONE: u8 = 0;
pub const UNIT_ENUM: u8 = 1;
pub const UNIT_DB: u8 = 2;
pub const UNIT_DB_MIN_INF: u8 = 3;
pub const UNIT_PERCENT: u8 = 4;
pub const UNIT_HZ: u8 = 5;
pub const UNIT_SEMITONES: u8 = 6;
pub const UNIT_CENTS: u8 = 7;
pub const UNIT_MS: u8 = 8;
pub const UNIT_SECONDS: u8 = 9;
pub const UNIT_FRAMES: u8 = 10;
pub const UNIT_MIDI_NOTE: u8 = 11;
pub const UNIT_MILLIVOLTS: u8 = 12;
pub const UNIT_VOLTS: u8 = 13;
pub const UNIT_BPM: u8 = 14;
pub const UNIT_DEGREES: u8 = 15;
pub const UNIT_HAS_STRINGS: u8 = 16;
pub const UNIT_CONFIRM: u8 = 17;
pub const UNIT_STRING: u8 = 18;
pub const UNIT_AUDIO_INPUT: u8 = 100;
pub const UNIT_CV_INPUT: u8 = 101;
pub const UNIT_AUDIO_OUTPUT: u8 = 102;
pub const UNIT_CV_OUTPUT: u8 = 103;
pub const UNIT_OUTPUT_MODE: u8 = 104;
pub const SCALING_NONE: u8 = 0;
pub const SCALING_10: u8 = 1;
pub const SCALING_100: u8 = 2;
pub const SCALING_1000: u8 = 3;
pub const TYPE_GENERIC: i32 = 0;
pub const TYPE_SECONDS: i32 = 1;
pub const TYPE_BOOLEAN: i32 = 2;
pub const TAG_INSTRUMENT: u32 = 1 << 0;
pub const TAG_EFFECT: u32 = 1 << 1;
pub const TAG_REVERB: u32 = 1 << 2;
pub const TAG_DELAY: u32 = 1 << 3;
pub const TAG_UTILITY: u32 = 1 << 4;
pub const TAG_FILTER_EQ: u32 = 1 << 5;
pub const BUTTON_1: u32 = 1 << 0;
pub const BUTTON_2: u32 = 1 << 1;
pub const BUTTON_3: u32 = 1 << 2;
pub const BUTTON_4: u32 = 1 << 3;
pub const POT_BUTTON_L: u32 = 1 << 4;
pub const POT_BUTTON_C: u32 = 1 << 5;
pub const POT_BUTTON_R: u32 = 1 << 6;
pub const ENCODER_BUTTON_L: u32 = 1 << 7;
pub const ENCODER_BUTTON_R: u32 = 1 << 8;
pub const ENCODER_L: u32 = 1 << 9;
pub const ENCODER_R: u32 = 1 << 10;
pub const POT_L: u32 = 1 << 11;
pub const POT_C: u32 = 1 << 12;
pub const POT_R: u32 = 1 << 13;
pub const PARAMETER_UI_PREFIX_SIZE: usize = 16;
pub const PARAMETER_STRING_SIZE: usize = 64;
pub const SCREEN_BYTES: usize = 128 * 64;
pub type NtFrame = [f32; 2];
pub type NtFloat3 = [f32; 3];

pub const fn multichar(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtGlobals {
    pub sample_rate: u32,
    pub max_frames_per_step: u32,
    pub work_buffer: *mut f32,
    pub work_buffer_size_bytes: u32,
    pub stream_size_bytes: u32,
    pub stream_buffer_size_bytes: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtStaticRequirements {
    pub dram: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtStaticMemoryPtrs {
    pub dram: *mut u8,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtAlgorithmRequirements {
    pub num_parameters: u32,
    pub sram: u32,
    pub dram: u32,
    pub dtc: u32,
    pub itc: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtAlgorithmMemoryPtrs {
    pub sram: *mut u8,
    pub dram: *mut u8,
    pub dtc: *mut u8,
    pub itc: *mut u8,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtParameter {
    pub name: *const c_char,
    pub min: i16,
    pub max: i16,
    pub default: i16,
    pub unit: u8,
    pub scaling: u8,
    pub enum_strings: *const *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtParameterPage {
    pub name: *const c_char,
    pub num_params: u8,
    pub group: u8,
    pub unused: [u8; 2],
    pub params: *const u8,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtParameterPages {
    pub num_pages: u32,
    pub pages: *const NtParameterPage,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtAlgorithm {
    pub parameters: *const NtParameter,
    pub parameter_pages: *const NtParameterPages,
    pub v_including_common: *const i16,
    pub v: *const i16,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtSpecification {
    pub name: *const c_char,
    pub min: i32,
    pub max: i32,
    pub default: i32,
    pub kind: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtUiData {
    pub pots: NtFloat3,
    pub controls: u16,
    pub last_buttons: u16,
    pub encoders: [i8; 2],
    pub unused: [u8; 2],
}
#[repr(C)]
pub struct NtSlot {
    ref_con: *mut core::ffi::c_void,
}
impl NtSlot {
    /// Equivalent to the C++ slot's empty constructor. Populate with NT_getSlot.
    pub const EMPTY: Self = Self {
        ref_con: core::ptr::null_mut(),
    };
}
#[repr(C)]
pub struct NtJsonStream {
    _private: [u8; 0],
}
#[repr(C)]
pub struct NtJsonParse {
    _private: [u8; 0],
}

pub type CalculateStaticRequirements = unsafe extern "C" fn(*mut NtStaticRequirements);
pub type Initialise = unsafe extern "C" fn(*mut NtStaticMemoryPtrs, *const NtStaticRequirements);
pub type CalculateRequirements = unsafe extern "C" fn(*mut NtAlgorithmRequirements, *const i32);
pub type Construct = unsafe extern "C" fn(
    *const NtAlgorithmMemoryPtrs,
    *const NtAlgorithmRequirements,
    *const i32,
) -> *mut NtAlgorithm;
pub type ParameterChanged = unsafe extern "C" fn(*mut NtAlgorithm, i32);
pub type Step = unsafe extern "C" fn(*mut NtAlgorithm, *mut f32, i32);
pub type Draw = unsafe extern "C" fn(*mut NtAlgorithm) -> bool;
pub type MidiRealtime = unsafe extern "C" fn(*mut NtAlgorithm, u8);
pub type MidiMessage = unsafe extern "C" fn(*mut NtAlgorithm, u8, u8, u8);
pub type HasCustomUi = unsafe extern "C" fn(*mut NtAlgorithm) -> u32;
pub type CustomUi = unsafe extern "C" fn(*mut NtAlgorithm, *const NtUiData);
pub type SetupUi = unsafe extern "C" fn(*mut NtAlgorithm, *mut NtFloat3);
pub type Serialise = unsafe extern "C" fn(*mut NtAlgorithm, *mut NtJsonStream);
pub type Deserialise = unsafe extern "C" fn(*mut NtAlgorithm, *mut NtJsonParse) -> bool;
pub type MidiSysEx = unsafe extern "C" fn(*const u8, u32);
pub type ParameterUiPrefix = unsafe extern "C" fn(*mut NtAlgorithm, i32, *mut c_char) -> i32;
pub type ParameterString = unsafe extern "C" fn(*mut NtAlgorithm, i32, i32, *mut c_char) -> i32;
pub type StringParameterBuffer =
    unsafe extern "C" fn(*mut NtAlgorithm, i32, *mut *mut c_char) -> i32;
pub type PluginEntry = unsafe extern "C" fn(u8, u32) -> usize;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NtFactory {
    pub guid: u32,
    pub name: *const c_char,
    pub description: *const c_char,
    pub num_specifications: u32,
    pub specifications: *const NtSpecification,
    pub calculate_static_requirements: Option<CalculateStaticRequirements>,
    pub initialise: Option<Initialise>,
    pub calculate_requirements: Option<CalculateRequirements>,
    pub construct: Option<Construct>,
    pub parameter_changed: Option<ParameterChanged>,
    pub step: Option<Step>,
    pub draw: Option<Draw>,
    pub midi_realtime: Option<MidiRealtime>,
    pub midi_message: Option<MidiMessage>,
    pub tags: u32,
    pub has_custom_ui: Option<HasCustomUi>,
    pub custom_ui: Option<CustomUi>,
    pub setup_ui: Option<SetupUi>,
    pub serialise: Option<Serialise>,
    pub deserialise: Option<Deserialise>,
    pub midi_sysex: Option<MidiSysEx>,
    pub parameter_ui_prefix: Option<ParameterUiPrefix>,
    pub parameter_string: Option<ParameterString>,
    pub string_parameter_buffer: Option<StringParameterBuffer>,
}

/// Rust counterpart to the `NT_PARAMETER_IO()` C macro.
/// This expands in the calling crate and emits no API-crate code.
#[macro_export]
macro_rules! nt_parameter_io {
    ($name:expr, $min:expr, $default:expr, $unit:expr) => {
        $crate::NtParameter {
            name: $name,
            min: $min,
            max: $crate::LAST_BUS as i16,
            default: $default,
            unit: $unit,
            scaling: $crate::SCALING_NONE,
            enum_strings: core::ptr::null(),
        }
    };
}

/// Rust counterpart to the `NT_PARAMETER_OUTPUT_MODE()` C macro.
#[macro_export]
macro_rules! nt_parameter_output_mode {
    ($name:expr) => {
        $crate::NtParameter {
            name: $name,
            min: 0,
            max: 1,
            default: 0,
            unit: $crate::UNIT_OUTPUT_MODE,
            scaling: $crate::SCALING_NONE,
            enum_strings: core::ptr::null(),
        }
    };
}

/// Loader-safe access to `NT_globals`, with no emitted Rust helper function.
#[macro_export]
macro_rules! nt_globals {
    () => {{
        unsafe { &*$crate::rust_nt_globals() }
    }};
}

unsafe impl Sync for NtGlobals {}
unsafe impl Sync for NtParameter {}
unsafe impl Sync for NtParameterPage {}
unsafe impl Sync for NtParameterPages {}
unsafe impl Sync for NtSpecification {}
unsafe impl Sync for NtFactory {}

unsafe extern "C" {
    pub fn NT_setParameterRange(ptr: *mut NtParameter, init: f32, min: f32, max: f32, step: f32);
    #[link_name = "NT_globals"]
    pub static NT_GLOBALS: NtGlobals;
    pub fn NT_getCpuCycleCount() -> u32;
    pub fn NT_algorithmIndex(algorithm: *const NtAlgorithm) -> i32;
    pub fn NT_algorithmCount() -> u32;
    pub fn NT_getSlot(slot: *mut NtSlot, index: u32) -> bool;
    pub fn NT_setParameterFromAudio(algorithm_index: u32, parameter: u32, value: i16);
    pub fn NT_setParameterFromUi(algorithm_index: u32, parameter: u32, value: i16);
    pub fn NT_setParameterGrayedOut(algorithm_index: u32, parameter: u32, gray: bool);
    pub fn NT_parameterOffset() -> u32;
    pub fn NT_updateParameterDefinition(algorithm_index: u32, parameter_index: u32);
    pub fn NT_updateParameterPages(algorithm_index: u32);
    pub fn NT_requestSetupUi();
    #[link_name = "NT_screen"]
    pub static mut NT_SCREEN: [u8; SCREEN_BYTES];
    pub fn NT_drawText(x: i32, y: i32, text: *const c_char, colour: i32, align: u8, size: u8);
    pub fn NT_drawShapeI(shape: u8, x0: i32, y0: i32, x1: i32, y1: i32, colour: i32);
    pub fn NT_drawShapeF(shape: u8, x0: f32, y0: f32, x1: f32, y1: f32, colour: f32);
    pub fn NT_intToString(buffer: *mut c_char, value: i32) -> i32;
    pub fn NT_floatToString(buffer: *mut c_char, value: f32, decimal_places: i32) -> i32;
    pub fn NT_sendMidiByte(destination: u32, b0: u8);
    pub fn NT_sendMidi2ByteMessage(destination: u32, b0: u8, b1: u8);
    pub fn NT_sendMidi3ByteMessage(destination: u32, b0: u8, b1: u8, b2: u8);
    pub fn NT_sendMidiSysEx(destination: u32, data: *const u8, count: u32, end: bool);
    pub fn exp2f(x: f32) -> f32;
    pub fn rust_nt_globals() -> *const NtGlobals;
    pub fn rust_nt_screen() -> *mut u8;
}
