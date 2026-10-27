//! Native simulated host for the reusable adapters, independently of any plugin.
extern crate std;
use super::*;
use crate::raw::*;
use core::{
    ffi::{c_char, CStr},
    ptr,
};
use std::{boxed::Box, cell::Cell, format, string::String, vec::Vec};

std::thread_local! {
    static REPORTED: Cell<Option<i32>> = const { Cell::new(None) };
    static FORMAT_CALLS: Cell<usize> = const { Cell::new(0) };
    static CURRENT: Cell<*mut Host> = const { Cell::new(ptr::null_mut()) };
}

unsafe fn write_text(buffer: *mut c_char, text: String) -> i32 {
    // SAFETY: test formatters receive the buffers sized by the public API.
    unsafe {
        ptr::copy_nonoverlapping(text.as_ptr(), buffer.cast(), text.len());
        *buffer.add(text.len()) = 0;
    }
    FORMAT_CALLS.with(|calls| calls.set(calls.get() + 1));
    REPORTED.with(|reported| reported.get().unwrap_or(text.len() as i32))
}
#[no_mangle]
unsafe extern "C" fn NT_intToString(buffer: *mut c_char, value: i32) -> i32 {
    unsafe { write_text(buffer, format!("{value}")) }
}
#[no_mangle]
unsafe extern "C" fn NT_floatToString(buffer: *mut c_char, value: f32, decimals: i32) -> i32 {
    unsafe { write_text(buffer, format!("{:.*}", decimals as usize, value)) }
}

#[test]
fn parameter_groups_check_whole_ranges_and_keep_local_scaling() {
    let definition = metadata::Definition::new(c"Value", 0, 3000, 0, UNIT_NONE, SCALING_1000, None)
        .unwrap()
        .raw();
    let definitions = [definition; 4];
    let values = [1000, 2000, 3000, 0];
    let parameters = parameters::ParameterValues::new(&values, &definitions);
    let group = parameters.group::<2>(1).unwrap();
    assert_eq!(group.len(), 2);
    assert_eq!(group.value(0), Some(2000));
    assert_eq!(group.scaled(1), Some(3.0));
    assert!(group.value(2).is_none());
    assert!(parameters.group::<2>(3).is_none());
    assert!(parameters.group::<2>(usize::MAX).is_none());
    assert!(parameters.group::<0>(4).unwrap().is_empty());
    assert!(parameters.group::<0>(5).is_none());
    let short_definitions = parameters::ParameterValues::new(&values, &definitions[..1]);
    assert!(short_definitions.group::<2>(0).is_none());
}

#[test]
fn compact_names_preserve_validated_definition_fields() {
    let name = metadata::Name::new(c"Renamed");
    assert_eq!(
        core::mem::size_of_val(&name),
        core::mem::size_of::<*const c_char>()
    );
    let original =
        metadata::Definition::new(c"Original", -10, 20, 5, UNIT_NONE, SCALING_10, None).unwrap();
    let renamed = original.with_name(name).raw();
    assert_eq!(renamed.name, c"Renamed".as_ptr());
    assert_eq!(
        (renamed.min, renamed.max, renamed.default, renamed.scaling),
        (-10, 20, 5, SCALING_10)
    );
}

#[test]
fn owned_text_tracks_termination_and_rejects_bad_lengths() {
    let mut text = text::TextBuffer::<64>::new();
    assert_eq!(text.as_c_str(), c"");
    assert!(text.format_float(b"L: ", -10.0, 3));
    assert_eq!(text.as_c_str(), c"L: -10.000");
    assert!(text.format_float(b"x\0hidden", 1.0, 3));
    assert_eq!(text.as_c_str(), c"x");
    for reported in [-1, i32::MAX, 50, 0] {
        REPORTED.with(|value| value.set(Some(reported)));
        let success = text.format_float(b"L: ", 1.0, 3);
        REPORTED.with(|value| value.set(None));
        assert_eq!(success, reported == 50 || reported == 0);
        assert_eq!(
            text.as_c_str(),
            if reported == 50 { c"L: 1.000" } else { c"L: " }
        );
    }
    assert!(!text.format_float(&[b'x'; 64], 1.0, 3));
    assert_eq!(text.as_c_str(), c"");
    let mut small = text::TextBuffer::<4>::new();
    assert!(!small.format_float(b"L: ", 1.0, 3));
    assert_eq!(small.as_c_str(), c"L: ");
    let mut minimum = text::TextBuffer::<1>::new();
    assert!(!minimum.format_float(b"", 1.0, 3));
    assert_eq!(minimum.as_c_str(), c"");
}

#[test]
fn formatting_checks_capacity_before_calling_the_host() {
    FORMAT_CALLS.with(|calls| calls.set(0));
    assert_eq!(text::format_integer(&mut [1; 11], i32::MIN), None);
    assert_eq!(text::format_float(&mut [1; 44], f32::MIN, 3), None);
    assert_eq!(FORMAT_CALLS.with(|calls| calls.get()), 0);
    let mut integer = [0xa5; 14];
    assert_eq!(
        text::format_integer(&mut integer[1..13], i32::MIN),
        Some(11)
    );
    assert_eq!(&integer[1..13], b"-2147483648\0");
    assert_eq!((integer[0], integer[13]), (0xa5, 0xa5));
    for value in [f32::MIN, f32::MAX, f32::NAN, f32::INFINITY] {
        let mut buffer = [0xa5; 66];
        assert!(text::format_float(&mut buffer[1..65], value, 3).is_some());
        assert_eq!((buffer[0], buffer[65]), (0xa5, 0xa5));
    }
}

#[test]
fn formatting_rejects_malformed_host_lengths() {
    for bad in [-1, 64, i32::MAX] {
        REPORTED.with(|reported| reported.set(Some(bad)));
        let mut bytes = [1; 64];
        assert_eq!(text::format_integer(&mut bytes, 5), None);
        assert_eq!(bytes[0], 0);
        assert_eq!(text::format_float(&mut bytes, 5.0, 3), None);
        assert_eq!(bytes[0], 0);
        REPORTED.with(|reported| reported.set(None));
    }
}

struct Host {
    algorithm: NtAlgorithm,
    definitions: [NtParameter; 2],
    values: [i16; 2],
    index: i32,
    last_set: Option<(bool, u32, i16)>,
    published_min: i16,
    gray: Option<(u32, bool)>,
}
impl Host {
    fn new() -> Box<Self> {
        let definition = NtParameter {
            name: c"Test".as_ptr(),
            min: -5000,
            max: 5000,
            default: 0,
            unit: UNIT_VOLTS,
            scaling: SCALING_1000,
            enum_strings: ptr::null(),
        };
        let mut host = Box::new(Self {
            algorithm: NtAlgorithm {
                parameters: ptr::null(),
                parameter_pages: ptr::null(),
                v: ptr::null(),
                v_including_common: ptr::null(),
            },
            definitions: [definition; 2],
            values: [1250, 0],
            index: 3,
            last_set: None,
            published_min: 0,
            gray: None,
        });
        host.algorithm.v = host.values.as_ptr();
        host.algorithm.parameters = host.definitions.as_ptr();
        CURRENT.with(|current| current.set(&mut *host));
        host
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(ptr::null_mut()));
    }
}
#[no_mangle]
unsafe extern "C" fn NT_algorithmIndex(_: *const NtAlgorithm) -> i32 {
    unsafe { (*CURRENT.with(|current| current.get())).index }
}
#[no_mangle]
unsafe extern "C" fn NT_parameterOffset() -> u32 {
    7
}
unsafe fn record_set(ui: bool, index: u32, parameter: u32, value: i16) {
    let host = unsafe { &mut *CURRENT.with(|current| current.get()) };
    assert_eq!(index, 3);
    host.last_set = Some((ui, parameter, value));
    host.values[(parameter - 7) as usize] = value;
}
#[no_mangle]
unsafe extern "C" fn NT_setParameterFromAudio(index: u32, p: u32, value: i16) {
    unsafe {
        record_set(false, index, p, value);
    }
}
#[no_mangle]
unsafe extern "C" fn NT_setParameterFromUi(index: u32, p: u32, value: i16) {
    unsafe {
        record_set(true, index, p, value);
    }
}
#[no_mangle]
unsafe extern "C" fn NT_setParameterGrayedOut(index: u32, p: u32, gray: bool) {
    let host = unsafe { &mut *CURRENT.with(|current| current.get()) };
    assert_eq!(index, 3);
    host.gray = Some((p, gray));
}
#[no_mangle]
unsafe extern "C" fn NT_updateParameterDefinition(index: u32, p: u32) {
    let host = unsafe { &mut *CURRENT.with(|current| current.get()) };
    assert_eq!(index, 3);
    host.published_min = host.definitions[p as usize].min;
    // Simulate a nested host update; an outer delayed write would overwrite it.
    host.definitions[p as usize].max = 4321;
}

#[test]
fn parameter_reads_are_copied_scaled_and_bounded() {
    let mut host = Host::new();
    let parameters = unsafe {
        parameters::Parameters::<parameters::ReadOnly>::from_raw(
            &mut host.algorithm,
            host.definitions.as_mut_ptr(),
            2,
        )
    };
    assert_eq!(parameters.value(0), Some(1250));
    assert_eq!(parameters.scaled(0), Some(1.25));
    for bad in [2, usize::MAX] {
        assert!(parameters.value(bad).is_none());
        assert!(parameters.definition(bad).is_none());
        assert!(parameters.scaled(bad).is_none());
    }
}

#[test]
fn parameter_views_expose_safe_information_and_check_indices() {
    let host = Host::new();
    let values = parameters::ParameterValues::new(&host.values, &host.definitions);
    assert_eq!(values.len(), 2);
    assert_eq!(values.scaled(0), Some(1.25));
    assert_eq!(values.definition(0).unwrap().name, c"Test");
    for bad in [2, usize::MAX] {
        assert!(values.value(bad).is_none());
        assert!(values.scaled(bad).is_none());
        assert!(values.definition(bad).is_none());
    }
}

#[test]
fn parameter_setters_use_correct_context_offsets_and_publish_before_notification() {
    let mut host = Host::new();
    let mut parameters = unsafe {
        parameters::Parameters::<parameters::Audio>::from_raw(
            &mut host.algorithm,
            host.definitions.as_mut_ptr(),
            2,
        )
    };
    assert!(parameters.set(2, 5).is_none());
    assert!(parameters.set(1, 17).is_some());
    assert_eq!(host.last_set, Some((false, 8, 17)));
    assert!(parameters.gray(1, true).is_some());
    assert_eq!(host.gray, Some((8, true)));
    assert!(parameters
        .update_definition(0, |definition| definition.min = -1234)
        .is_some());
    assert_eq!(host.published_min, -1234);
    assert_eq!(host.definitions[0].max, 4321);
    let mut ui = unsafe {
        parameters::Parameters::<parameters::Ui>::from_raw(
            &mut host.algorithm,
            host.definitions.as_mut_ptr(),
            2,
        )
    };
    assert!(ui.set(0, 42).is_some());
    assert_eq!(host.last_set, Some((true, 7, 42)));
    host.index = -1;
    assert!(ui.set(0, 99).is_none());
    assert_eq!(host.values[0], 42);
}

#[test]
fn enum_tables_keep_static_c_string_addresses() {
    static NAMES: metadata::EnumStrings<2> = metadata::EnumStrings::new([c"No", c"Yes"]);
    unsafe {
        assert_eq!(CStr::from_ptr(*NAMES.as_ptr()), c"No");
        assert_eq!(CStr::from_ptr(*NAMES.as_ptr().add(1)), c"Yes");
    }
}

#[derive(Default)]
struct Json {
    members: Vec<(String, bool)>,
    name: String,
    position: usize,
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_add_member_name(raw: *mut NtJsonStream, name: *const c_char) {
    unsafe {
        (*raw.cast::<Json>()).name = CStr::from_ptr(name).to_str().unwrap().into();
    }
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_add_boolean(raw: *mut NtJsonStream, value: bool) {
    let json = unsafe { &mut *raw.cast::<Json>() };
    json.members.push((json.name.clone(), value));
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_number_of_object_members(
    raw: *mut NtJsonParse,
    count: *mut i32,
) -> bool {
    unsafe {
        *count = (*raw.cast::<Json>()).members.len() as i32;
    }
    true
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_match_name(raw: *mut NtJsonParse, name: *const c_char) -> bool {
    let json = unsafe { &*raw.cast::<Json>() };
    json.members[json.position].0 == unsafe { CStr::from_ptr(name) }.to_str().unwrap()
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_boolean(raw: *mut NtJsonParse, value: *mut bool) -> bool {
    let json = unsafe { &mut *raw.cast::<Json>() };
    unsafe {
        *value = json.members[json.position].1;
    }
    json.position += 1;
    true
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_skip_member(raw: *mut NtJsonParse) -> bool {
    unsafe {
        (*raw.cast::<Json>()).position += 1;
    }
    true
}

#[test]
fn json_adapters_round_trip_members_through_host_bridges() {
    use json::{Reader, Writer};
    let mut state = Json::default();
    let mut writer = unsafe { json::JsonWriter::new((&mut state as *mut Json).cast()) };
    writer.boolean_member(c"First", true);
    writer.boolean_member(c"Second", false);
    let mut reader = unsafe { json::JsonReader::new((&mut state as *mut Json).cast()) };
    assert_eq!(reader.member_count(), Some(2));
    assert!(reader.matches(c"First"));
    let mut value = false;
    assert!(reader.boolean(&mut value));
    assert!(value);
    assert!(reader.matches(c"Second"));
    assert!(reader.skip());
    assert_eq!(state.position, 2);
}
