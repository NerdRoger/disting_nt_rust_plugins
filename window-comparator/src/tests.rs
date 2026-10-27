extern crate std;
use distingnt_sys::*;
unsafe fn context<'a>(a: *mut Algorithm) -> Callback<'a, distingnt_api::api::parameters::Audio> {
    distingnt_api::api::plugin::Context::from_raw(a)
}
unsafe fn update_definition(
    a: *mut Algorithm,
    _: u32,
    p: usize,
    change: impl FnOnce(&mut distingnt_api::api::parameters::Bounds),
) {
    let mut callback = context(a);
    assert!(callback.parameters().update_definition(p, change).is_some());
}
use core::{ffi::c_char, ptr};
#[test]
fn api_v14_routes_input_gates_and_triggers_above_bus_28() {
    unsafe {
        let mut instance = Instance::new(1);
        instance.values[4 + cp::INPUT] = LAST_BUS as i16;
        instance.values[4 + cp::INSIDE_GATE] = 63;
        instance.values[4 + cp::OUTSIDE_GATE] = 62;
        instance.values[4 + cp::ENTER_TRIGGER] = 61;
        instance.values[4 + cp::EXIT_TRIGGER] = 60;
        for frames in [4, 16, 48, 128] {
            let mut samples = vec![-123.0; LAST_BUS * frames];
            for (input, inside) in [(0.0, true), (2.0, false), (0.0, true)] {
                samples[(LAST_BUS - 1) * frames..].fill(input);
                Bridge::step(instance.a.cast(), samples.as_mut_ptr(), (frames / 4) as i32);
                assert_eq!(samples[62 * frames], if inside { 10.0 } else { 0.0 });
                assert_eq!(samples[61 * frames], if inside { 0.0 } else { 10.0 });
                if !inside {
                    assert_eq!(samples[59 * frames], 10.0);
                }
            }
            assert_eq!(samples[60 * frames], 10.0);
        }
        assert_eq!(pluginEntry(SELECTOR_VERSION, 0), API_VERSION_14);
        assert!(factory().string_parameter_buffer.is_none());
        assert!(factory().calculate_static_requirements.is_none());
        assert!(factory().initialise.is_none());
    }
}

#[test]
fn constructor_rejects_invalid_storage_without_writing() {
    unsafe {
        let n = 8;
        let mut req = core::mem::zeroed();
        Bridge::calculate_requirements(&mut req, &n);
        let size = req.sram as usize;
        let alignment = Bridge::alignment(&[n]).unwrap();
        let allocation = Layout::from_size_align(size + alignment, alignment).unwrap();
        let ram = alloc_zeroed(allocation);
        assert!(!ram.is_null());
        ptr::write_bytes(ram, 0xa5, allocation.size());
        let mut mem = NtAlgorithmMemoryPtrs {
            sram: ram,
            dram: ptr::null_mut(),
            dtc: ptr::null_mut(),
            itc: ptr::null_mut(),
        };
        req.sram -= 1;
        assert!(Bridge::construct(&mem, &req, &n).is_null());
        req.sram += 1;
        mem.sram = ram.add(1);
        assert!(Bridge::construct(&mem, &req, &n).is_null());
        mem.sram = ptr::null_mut();
        assert!(Bridge::construct(&mem, &req, &n).is_null());
        mem.sram = ram;
        for invalid in [-1, 0, 9, i32::MAX] {
            assert!(Bridge::construct(&mem, &req, &invalid).is_null());
        }
        assert!(core::slice::from_raw_parts(ram, allocation.size())
            .iter()
            .all(|&b| b == 0xa5));
        dealloc(ram, allocation);
    }
}
#[test]
fn safe_comparator_preserves_boundaries_first_step_and_nan() {
    use engine::{Comparator, Input};
    let mut state = Comparator::default();
    let mut input = Input {
        sample: -1.0,
        scale: 100.0,
        offset: 0.0,
        range_min: -5.0,
        range_max: 5.0,
        left: -1.0,
        right: 1.0,
    };
    let first = state.process(input, true);
    assert!(first.inside && !first.entered && !first.exited);
    input.sample = 1.0;
    assert!(state.process(input, false).inside); // inclusive upper bound
    input.sample = 1.01;
    assert!(state.process(input, false).exited);
    input.sample = -20.0;
    assert!(!state.process(input, false).inside);
    assert_eq!(state.current_value, -5.0);
    input.sample = f32::NAN;
    assert!(!state.process(input, false).inside);
    assert!(state.current_value.is_nan());
    input.sample = 4.0;
    input.range_min = 0.0;
    input.range_max = 0.0;
    input.left = 0.0;
    input.right = 0.0;
    assert!(state.process(input, false).entered);
    assert_eq!(state.current_value, 0.0);
}
use super::*;
use std::fmt::Write;
use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    cell::{Cell, RefCell},
    ffi::CStr,
    format,
    string::String,
    vec,
    vec::Vec,
};

std::thread_local! {
    static ALGORITHM_INDEX: Cell<i32> = const { Cell::new(0) };
    static CURRENT: Cell<*mut Algorithm> = const { Cell::new(ptr::null_mut()) };
    static DRAW: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static UPDATES: RefCell<Vec<(u32, i16)>> = const { RefCell::new(Vec::new()) };
    static FORMAT_LENGTH: Cell<Option<i32>> = const { Cell::new(None) };
    static INT_FORMAT_LENGTH: Cell<Option<i32>> = const { Cell::new(None) };
    static DEFINITION_HOOK: Cell<Option<unsafe fn(*mut Algorithm, u32)>> = const { Cell::new(None) };
}

unsafe fn snapshot(a: *mut Algorithm, n: i32, phase: &str, trace: &mut String) {
    DRAW.with(|d| d.borrow_mut().clear());
    Bridge::draw(a.cast());
    let mut hash = 2166136261u32;
    DRAW.with(|d| {
        for line in d.borrow().iter() {
            for b in line.as_bytes() {
                hash = (hash ^ *b as u32).wrapping_mul(16777619);
            }
        }
    });
    let mut json = Json::default();
    Bridge::serialise(a.cast(), (&mut json as *mut Json).cast());
    writeln!(
        trace,
        "D {n} {phase} {hash:08x} {} {}",
        json.members[0].1 as u8, json.members[1].1 as u8
    )
    .unwrap();
}

#[test]
#[ignore = "run make test to generate traces from the original C++ plugin first"]
fn matches_original_cpp_trace() {
    unsafe {
        let mut trace = String::new();
        for n in 1..=8 {
            let mut instance = Instance::new(n);
            let a = instance.a;
            for p in 0..instance.values.len() {
                let d = &*(*a).parameters.add(p);
                writeln!(
                    trace,
                    "P {n} {p} {} {} {} {} {} {}",
                    str_at(d.name),
                    d.min,
                    d.max,
                    d.default,
                    d.unit,
                    d.scaling
                )
                .unwrap();
            }
            for p in 0..(*(*a).parameter_pages).num_pages as usize {
                let page = &*(*(*a).parameter_pages).pages.add(p);
                write!(trace, "G {n} {} {}", str_at(page.name), page.group).unwrap();
                for j in 0..page.num_params as usize {
                    write!(trace, " {}", *page.params.add(j)).unwrap();
                }
                trace.push('\n');
            }
            for (p, v) in [(5, -2000), (7, 0), (0, -10), (1, 10), (8, 4000)] {
                instance.change(p, v);
            }
            for p in 0..instance.values.len() {
                let d = &*(*a).parameters.add(p);
                writeln!(
                    trace,
                    "V {n} {p} {} {} {}",
                    instance.values[p], d.min, d.max
                )
                .unwrap();
            }
            for ch in 0..n as usize {
                let o = channel_offset(ch);
                instance.values[o] = if ch % 3 == 2 { 2 } else { 1 };
                instance.values[o + 1] = -1000 + ch as i16 * 100;
                instance.values[o + 2] = 1000 + ch as i16 * 100;
                instance.values[o + 5] = (ch % 2) as i16;
                instance.values[o + 6] = 5000;
                instance.values[o + 7] = ch as i16 * 10;
                for j in 0..4 {
                    instance.values[o + 11 + j] = 2 + j as i16;
                }
            }
            if n > 1 {
                for j in 0..12 {
                    instance.values[19 + j] = 6 + j as i16;
                }
                instance.values[31] = 18;
                instance.values[32] = 19;
                for ch in 1..n as usize {
                    for j in 0..4 {
                        instance.values[channel_offset(ch) + 15 + j] = 20 + j as i16;
                    }
                }
            }
            let inputs = [-10.0, -3.0, -1.0, 0.0, 1.0, 3.0, 10.0, 0.0];
            for tick in 0..32 {
                let frames = [4, 16, 48, 128][tick % 4];
                let mut buses = vec![-123.0; LAST_BUS * frames];
                buses[..frames].fill(inputs[tick % 8]);
                Bridge::step(a.cast(), buses.as_mut_ptr(), (frames / 4) as i32);
                write!(trace, "B {n} {tick}").unwrap();
                for v in buses {
                    write!(
                        trace,
                        " {:08x}",
                        if v.is_nan() {
                            0x7fc00000u32
                        } else {
                            v.to_bits()
                        }
                    )
                    .unwrap();
                }
                trace.push('\n');
            }
            snapshot(a, n, "default", &mut trace);
            let mut ui = NtUiData {
                pots: [0.5; 3],
                controls: 0,
                last_buttons: 0,
                encoders: [0; 2],
                unused: [0; 2],
            };
            Bridge::custom_ui(a.cast(), &ui);
            ui.encoders[0] = 1;
            ui.controls = POT_L as u16;
            ui.pots[0] = 0.505;
            Bridge::custom_ui(a.cast(), &ui);
            snapshot(a, n, "pot", &mut trace);
            ui.encoders[0] = 0;
            ui.controls = ENCODER_BUTTON_R as u16;
            Bridge::custom_ui(a.cast(), &ui);
            ui.controls = 0;
            ui.last_buttons = ENCODER_BUTTON_R as u16;
            Bridge::custom_ui(a.cast(), &ui);
            snapshot(a, n, "locked", &mut trace);
            ui.controls = ENCODER_BUTTON_R as u16;
            ui.last_buttons = 0;
            Bridge::custom_ui(a.cast(), &ui);
            let mut buses = vec![0.0; LAST_BUS * 48];
            for _ in 0..300 {
                Bridge::step(a.cast(), buses.as_mut_ptr(), 12);
            }
            snapshot(a, n, "long", &mut trace);
            ui.controls = 0;
            ui.last_buttons = ENCODER_BUTTON_R as u16;
            Bridge::custom_ui(a.cast(), &ui);
            let mut json = Json {
                members: vec![
                    (String::from("Unknown"), true),
                    (String::from("BoundsEditMode"), true),
                    (String::from("Editable"), true),
                ],
                ..Json::default()
            };
            assert!(Bridge::deserialise(
                a.cast(),
                (&mut json as *mut Json).cast()
            ));
            snapshot(a, n, "restored", &mut trace);
        }
        let reference = std::fs::read_to_string("../target/window-cpp-reference.txt").unwrap();
        std::fs::write("../target/window-rust-reference.txt", &trace).unwrap();
        let expected: Vec<_> = reference.lines().collect();
        let actual: Vec<_> = trace.lines().collect();
        assert_eq!(actual.len(), expected.len());
        for (i, (e, a)) in expected.iter().zip(&actual).enumerate() {
            assert_eq!(a, e, "trace differs at line {}", i + 1);
        }
    }
}

struct Instance {
    a: *mut Algorithm,
    layout: Layout,
    values: Vec<i16>,
}
impl Instance {
    unsafe fn new(n: i32) -> Self {
        let mut req = core::mem::zeroed();
        Bridge::calculate_requirements(&mut req, &n);
        let layout =
            Layout::from_size_align(req.sram as usize, Bridge::alignment(&[n]).unwrap()).unwrap();
        let ram = alloc_zeroed(layout);
        assert!(!ram.is_null());
        // Construction must initialize everything it uses, even when the host
        // provides storage that was previously used by another instance.
        ptr::write_bytes(ram, 0xa5, layout.size());
        let mem = NtAlgorithmMemoryPtrs {
            sram: ram,
            dram: ptr::null_mut(),
            dtc: ptr::null_mut(),
            itc: ptr::null_mut(),
        };
        let a = Bridge::construct(&mem, &req, &n).cast::<Algorithm>();
        let values: Vec<i16> = (0..req.num_parameters as usize)
            .map(|p| (*(*a).parameters.add(p)).default)
            .collect();
        (*a).v = values.as_ptr();
        (*a).v_including_common = values.as_ptr();
        CURRENT.with(|v| v.set(a));
        Self { a, layout, values }
    }
    unsafe fn change(&mut self, p: usize, v: i16) {
        self.values[p] = v;
        Bridge::parameter_changed(self.a.cast(), p as i32);
    }
    unsafe fn step(&mut self, input: f32) -> Vec<f32> {
        let mut buses = vec![-123.0; LAST_BUS * 4];
        buses[..4].fill(input);
        Bridge::step(self.a.cast(), buses.as_mut_ptr(), 1);
        buses
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.a.cast(), self.layout);
        }
        CURRENT.with(|v| v.set(ptr::null_mut()));
    }
}

#[no_mangle]
unsafe extern "C" fn rust_nt_globals() -> *const NtGlobals {
    static G: NtGlobals = NtGlobals {
        sample_rate: 48000,
        max_frames_per_step: 128,
        work_buffer: ptr::null_mut(),
        work_buffer_size_bytes: 0,
        stream_size_bytes: 0,
        stream_buffer_size_bytes: 0,
    };
    &G
}
#[no_mangle]
unsafe extern "C" fn NT_algorithmIndex(_: *const NtAlgorithm) -> i32 {
    ALGORITHM_INDEX.with(|index| index.get())
}
#[no_mangle]
unsafe extern "C" fn NT_parameterOffset() -> u32 {
    0
}
#[no_mangle]
unsafe extern "C" fn NT_setParameterFromAudio(_: u32, p: u32, v: i16) {
    set(p, v);
}
#[no_mangle]
unsafe extern "C" fn NT_setParameterFromUi(_: u32, p: u32, v: i16) {
    set(p, v);
}
unsafe fn set(p: u32, v: i16) {
    UPDATES.with(|u| u.borrow_mut().push((p, v)));
    let a = CURRENT.with(|a| a.get());
    let def = &*(*a).parameters.add(p as usize);
    let v = v.max(def.min).min(def.max);
    let slot = (*a).v.add(p as usize).cast_mut();
    if *slot != v {
        *slot = v;
        Bridge::parameter_changed(a.cast(), p as i32);
    }
}
#[no_mangle]
unsafe extern "C" fn NT_updateParameterDefinition(_: u32, parameter: u32) {
    // Take the hook before reentry; no fixture borrow crosses the callback.
    if let Some(hook) = DEFINITION_HOOK.with(|hook| hook.take()) {
        hook(CURRENT.with(|current| current.get()), parameter);
    }
}
#[no_mangle]
unsafe extern "C" fn NT_setParameterGrayedOut(_: u32, _: u32, _: bool) {}
unsafe fn str_at(s: *const c_char) -> String {
    CStr::from_ptr(s).to_str().unwrap().into()
}
#[no_mangle]
unsafe extern "C" fn NT_drawText(
    x: i32,
    y: i32,
    s: *const c_char,
    color: i32,
    align: u8,
    size: u8,
) {
    let s = str_at(s);
    DRAW.with(|d| {
        d.borrow_mut()
            .push(format!("T {x} {y} {color} {align} {size} {s}"))
    });
}
#[no_mangle]
unsafe extern "C" fn NT_drawShapeI(kind: u8, x0: i32, y0: i32, x1: i32, y1: i32, color: i32) {
    DRAW.with(|d| {
        d.borrow_mut()
            .push(format!("S {kind} {x0} {y0} {x1} {y1} {color}"))
    });
}
unsafe fn write_string(buff: *mut c_char, text: String) -> i32 {
    ptr::copy_nonoverlapping(text.as_ptr(), buff.cast(), text.len());
    *buff.add(text.len()) = 0;
    text.len() as i32
}
#[no_mangle]
unsafe extern "C" fn NT_intToString(buff: *mut c_char, v: i32) -> i32 {
    let length = write_string(buff, format!("{v}"));
    INT_FORMAT_LENGTH.with(|reported| reported.get().unwrap_or(length))
}

#[test]
fn parameter_text_checks_reported_lengths_and_preserves_buffer_edges() {
    unsafe {
        let instance = Instance::new(1);
        for value in [i32::MIN, -10, 0, 10, i32::MAX] {
            let mut bytes = [0xa5u8; PARAMETER_STRING_SIZE + 2];
            let buffer = bytes.as_mut_ptr().add(1).cast();
            let length = Bridge::parameter_string(instance.a, gp::RANGE_MIN as i32, value, buffer);
            assert_eq!(str_at(buffer), format!("{value}V"));
            assert_eq!(length as usize, format!("{value}V").len());
            assert_eq!(bytes[0], 0xa5);
            assert_eq!(bytes[PARAMETER_STRING_SIZE + 1], 0xa5);
        }
        for invalid in [-1, (PARAMETER_STRING_SIZE - 1) as i32, i32::MAX] {
            INT_FORMAT_LENGTH.with(|reported| reported.set(Some(invalid)));
            let mut bytes = [0xa5u8; PARAMETER_STRING_SIZE + 2];
            let buffer = bytes.as_mut_ptr().add(1).cast();
            let length = Bridge::parameter_string(instance.a, gp::RANGE_MAX as i32, 5, buffer);
            INT_FORMAT_LENGTH.with(|reported| reported.set(None));
            assert_eq!(length, 0);
            assert_eq!(str_at(buffer), "");
            assert_eq!(bytes[0], 0xa5);
            assert_eq!(bytes[PARAMETER_STRING_SIZE + 1], 0xa5);
        }
    }
}

#[test]
fn definition_notification_publishes_before_reentry_and_preserves_nested_updates() {
    unsafe fn reenter(a: *mut Algorithm, p: u32) {
        assert_eq!((*(*a).parameters.add(p as usize)).max, 3000);
        update_definition(a, 0, p as usize, |definition| definition.max = 4000);
        set((gp::COUNT + cp::LEFT) as u32, -2000);
    }
    unsafe {
        let i = Instance::new(1);
        let p = gp::COUNT + cp::RIGHT;
        DEFINITION_HOOK.with(|hook| hook.set(Some(reenter)));
        update_definition(i.a, 0, p, |definition| definition.max = 3000);
        assert!(DEFINITION_HOOK.with(|hook| hook.get().is_none()));
        let definition = ptr::read((*i.a).parameters.add(p));
        assert_eq!(definition.max, 4000);
        assert_eq!(definition.min, -2000);
        assert_eq!(i.values[gp::COUNT + cp::CENTER], -500);
        assert_eq!(i.values[gp::COUNT + cp::WIDTH], 3000);
    }
}
#[no_mangle]
unsafe extern "C" fn NT_floatToString(buff: *mut c_char, v: f32, decimals: i32) -> i32 {
    let length = write_string(buff, format!("{:.*}", decimals as usize, v));
    FORMAT_LENGTH.with(|reported| reported.get().unwrap_or(length))
}

#[test]
fn drawing_text_is_terminated_even_with_malformed_reported_lengths() {
    use drawing::StatisticText;
    let text = StatisticText::format(b'L', -10.0);
    assert_eq!(text.as_c_str(), c"L: -10.000");
    let text = StatisticText::format(b'W', 20.0);
    assert_eq!(text.as_c_str(), c"W: 20.000");
    let text = StatisticText::format(b'X', f32::MAX);
    assert!(text.as_c_str().to_bytes().starts_with(b"X: "));
    assert!(text.as_c_str().to_bytes().len() < PARAMETER_STRING_SIZE);
    for invalid in [-1, i32::MAX] {
        FORMAT_LENGTH.with(|reported| reported.set(Some(invalid)));
        let text = StatisticText::format(b'L', -10.0);
        FORMAT_LENGTH.with(|reported| reported.set(None));
        assert_eq!(text.as_c_str(), c"L: ");
    }
}

#[derive(Default)]
struct Json {
    members: Vec<(String, bool)>,
    name: String,
    pos: usize,
    fail: bool,
    fail_at: Option<usize>,
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_add_member_name(s: *mut NtJsonStream, n: *const c_char) {
    (*s.cast::<Json>()).name = str_at(n);
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_add_boolean(s: *mut NtJsonStream, v: bool) {
    let s = &mut *s.cast::<Json>();
    s.members.push((s.name.clone(), v));
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_number_of_object_members(
    p: *mut NtJsonParse,
    n: *mut i32,
) -> bool {
    let p = &*p.cast::<Json>();
    *n = p.members.len() as i32;
    !p.fail
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_match_name(p: *mut NtJsonParse, n: *const c_char) -> bool {
    let p = &*p.cast::<Json>();
    p.members[p.pos].0 == str_at(n)
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_boolean(p: *mut NtJsonParse, v: *mut bool) -> bool {
    let p = &mut *p.cast::<Json>();
    if p.fail || p.fail_at == Some(p.pos) {
        return false;
    }
    *v = p.members[p.pos].1;
    p.pos += 1;
    true
}
#[no_mangle]
unsafe extern "C" fn rust_nt_json_skip_member(p: *mut NtJsonParse) -> bool {
    (*p.cast::<Json>()).pos += 1;
    true
}

#[test]
fn preset_callback_publishes_partial_state_on_later_parse_failure() {
    unsafe {
        for bounds_first in [false, true] {
            let i = Instance::new(1);
            let names = if bounds_first {
                ["BoundsEditMode", "Editable"]
            } else {
                ["Editable", "BoundsEditMode"]
            };
            let mut json = Json::default();
            json.members
                .extend(names.map(|name| (String::from(name), false)));
            json.fail_at = Some(1);
            assert!(!Bridge::deserialise(
                i.a.cast(),
                (&mut json as *mut Json).cast()
            ));
            assert_eq!(
                context(i.a).state().ui.preset().edit_mode.is_bounds(),
                !bounds_first
            );
            assert_eq!(context(i.a).state().ui.preset().editable, bounds_first);
            assert_eq!(json.pos, 1);
        }
    }
}

#[test]
fn all_channel_layouts_are_stable_and_complete() {
    unsafe {
        for n in 1..=8 {
            let i = Instance::new(n);
            let a = i.a;
            assert_eq!(
                i.values.len(),
                if n == 1 {
                    19
                } else {
                    33 + (n as usize - 1) * 19
                }
            );
            assert_eq!(
                (*(*a).parameter_pages).num_pages,
                if n == 1 { 2 } else { n as u32 + 3 }
            );
            let mut seen = vec![false; i.values.len()];
            for p in 0..(*(*a).parameter_pages).num_pages as usize {
                let page = &*(*(*a).parameter_pages).pages.add(p);
                for j in 0..page.num_params as usize {
                    let k = *page.params.add(j) as usize;
                    assert!(!seen[k]);
                    seen[k] = true;
                }
            }
            assert!(seen.iter().all(|x| *x));
            for ch in 0..n as usize {
                assert_eq!(
                    context(a).voices().get(ch).unwrap().offset(),
                    channel_offset(ch)
                );
            }
        }
    }
}
#[test]
fn later_channels_read_earlier_channels_bus_writes() {
    unsafe {
        let mut i = Instance::new(2);
        let second = channel_offset(1);
        i.values[gp::COUNT + cp::INSIDE_GATE] = 2;
        i.values[second + cp::INPUT] = 2;
        i.values[second + cp::INSIDE_GATE] = 3;
        i.values[second + cp::OUTSIDE_GATE] = 4;
        let out = i.step(0.0);
        assert_eq!(out[4], 10.0);
        assert_eq!(out[8], 0.0);
        assert_eq!(out[12], 10.0);
        assert_eq!(context(i.a).voices().get(1).unwrap().current_value(), 5.0);
    }
}

#[test]
fn processing_parameter_view_is_bounded() {
    unsafe {
        let instance = Instance::new(1);
        let mut callback = context(instance.a);
        let data = callback.processing();
        assert_eq!(data.parameters.len(), instance.values.len());
        for index in [data.parameters.len(), usize::MAX] {
            assert!(data.parameters.value(index).is_none());
            assert!(data.parameters.scaled(index).is_none());
        }
    }
}

#[test]
fn synchronization_guards_clear_when_a_host_setter_fails() {
    use engine::Guard;
    unsafe {
        for parameter in [cp::LEFT, cp::CENTER] {
            let instance = Instance::new(1);
            let mut callback = context(instance.a);
            ALGORITHM_INDEX.with(|index| index.set(-1));
            let result = parameter_changes::changed(&mut callback, gp::COUNT + parameter);
            ALGORITHM_INDEX.with(|index| index.set(0));
            assert!(result.is_none());
            let channel = &callback.voices()[0];
            assert!(!channel.guard(Guard::Bounds));
            assert!(!channel.guard(Guard::SizePosition));
            // Clearing the flag also allows the next valid update to succeed.
            assert!(parameter_changes::changed(&mut callback, gp::COUNT + parameter).is_some());
        }
    }
}

#[test]
fn processing_edges_stacking_overrides_and_empty_averages() {
    unsafe {
        let mut i = Instance::new(2);
        // A inside/outside gates and enter/exit triggers.
        for (p, bus) in [(15, 2), (16, 3), (17, 4), (18, 5)] {
            i.values[p] = bus;
        }
        i.values[33 + 11] = 2; // B stacks into A's inside gate.
        i.values[19] = 6;
        i.values[21] = 7;
        i.values[22] = 8;
        let out = i.step(0.0);
        assert_eq!(out[4], 10.0);
        assert_eq!(out[12], 0.0); // first step suppresses enter
        assert_eq!(out[20], 2.0);
        assert_eq!(out[24], 1.0);
        assert!(out[28].is_nan());
        let out = i.step(2.0);
        assert_eq!(out[4], 0.0);
        assert_eq!(out[8], 10.0);
        assert_eq!(out[16], 10.0);
        let out = i.step(2.0);
        assert_eq!(out[16], 0.0);
        i.values[33 + 5] = 1;
        i.values[33 + 6] = 0; // B sees zero despite A outside
        let out = i.step(2.0);
        assert_eq!(out[4], 10.0);
        assert_eq!(out[8], 10.0);
        assert_eq!(out[20], 1.0);
        let out = i.step(0.0);
        assert_eq!(out[12], 10.0);
        assert_eq!(out[24], 1.0);
    }
}
#[test]
fn parameter_updates_are_reentrant_and_keep_bounds_in_sync() {
    unsafe {
        let mut i = Instance::new(1);
        i.change(5, -2000);
        assert_eq!(i.values[7], -500);
        assert_eq!(i.values[8], 3000);
        assert_eq!((*(*i.a).parameters.add(6)).min, -2000);
        i.change(7, 0);
        assert_eq!(i.values[5], -1500);
        assert_eq!(i.values[6], 1500);
        i.change(0, -10);
        i.change(1, 10);
        assert_eq!((*(*i.a).parameters.add(5)).min, -10000);
        assert_eq!((*(*i.a).parameters.add(8)).max, 20000);
    }
}
#[test]
fn ui_short_long_presses_pots_and_json_state() {
    unsafe {
        let i = Instance::new(8);
        let a = i.a;
        let mut pots = [0.0; 3];
        Bridge::setup_ui(a.cast(), &mut pots);
        assert_eq!(pots, [0.5; 3]);
        let mut data = NtUiData {
            pots: [0.5; 3],
            controls: 0,
            last_buttons: 0,
            encoders: [0; 2],
            unused: [0; 2],
        };
        Bridge::custom_ui(a.cast(), &data); // prime pot manager
        context(a).state_mut().clock.advance(10 * 48);
        data.controls = ENCODER_BUTTON_R as u16;
        Bridge::custom_ui(a.cast(), &data);
        context(a).state_mut().clock.advance(90 * 48);
        data.controls = 0;
        data.last_buttons = ENCODER_BUTTON_R as u16;
        Bridge::custom_ui(a.cast(), &data);
        assert!(!context(a).state().ui.preset().editable);
        context(a).state_mut().clock.advance(100 * 48);
        data.controls = ENCODER_BUTTON_R as u16;
        data.last_buttons = 0;
        Bridge::custom_ui(a.cast(), &data);
        context(a).state_mut().clock.advance(300 * 48);
        DRAW.with(|d| d.borrow_mut().clear());
        Bridge::draw(a.cast());
        assert!(!context(a).state().ui.preset().edit_mode.is_bounds());
        data.controls = 0;
        data.last_buttons = ENCODER_BUTTON_R as u16;
        Bridge::custom_ui(a.cast(), &data);
        // Serialization round-trip, including an unknown member and malformed object.
        let mut json = Json::default();
        Bridge::serialise(a.cast(), (&mut json as *mut Json).cast());
        assert_eq!(
            json.members,
            vec![
                (String::from("BoundsEditMode"), false),
                (String::from("Editable"), false)
            ]
        );
        context(a).state_mut().ui.restore(preset::State {
            edit_mode: controls::EditMode::Bounds,
            editable: true,
        });
        json.members.insert(0, (String::from("Unknown"), true));
        assert!(Bridge::deserialise(
            a.cast(),
            (&mut json as *mut Json).cast()
        ));
        assert!(
            !context(a).state().ui.preset().edit_mode.is_bounds()
                && !context(a).state().ui.preset().editable
        );
        json.fail = true;
        assert!(!Bridge::deserialise(
            a.cast(),
            (&mut json as *mut Json).cast()
        ));
        // Unlock, select channel B and move left pot. Only B's center changes.
        context(a).state_mut().ui.restore(preset::State {
            editable: true,
            ..context(a).state().ui.preset()
        });
        data.last_buttons = 0;
        data.encoders = [1, 0];
        data.controls = POT_L as u16;
        data.pots[0] = 0.505;
        UPDATES.with(|u| u.borrow_mut().clear());
        Bridge::custom_ui(a.cast(), &data);
        assert!(UPDATES.with(|u| u.borrow().iter().any(|(p, _)| *p == 36)));
    }
}
// This is the simulated external C++ side, so it obtains callbacks from the
// published raw factory instead of using API-internal ABI functions.
unsafe fn factory() -> &'static NtFactory {
    &*(Framework::entry(SELECTOR_FACTORY_INFO, 0) as *const NtFactory)
}
struct Bridge;
impl Bridge {
    fn alignment(specs: &[i32]) -> Option<usize> {
        Framework::alignment(specs)
    }
    unsafe fn calculate_requirements(req: *mut NtAlgorithmRequirements, specs: *const i32) {
        (factory().calculate_requirements.unwrap())(req, specs)
    }
    unsafe fn construct(
        mem: *const NtAlgorithmMemoryPtrs,
        req: *const NtAlgorithmRequirements,
        specs: *const i32,
    ) -> *mut NtAlgorithm {
        (factory().construct.unwrap())(mem, req, specs)
    }
    unsafe fn parameter_changed(base: *mut NtAlgorithm, parameter: i32) {
        (factory().parameter_changed.unwrap())(base, parameter)
    }
    unsafe fn step(base: *mut NtAlgorithm, buses: *mut f32, frames: i32) {
        (factory().step.unwrap())(base, buses, frames)
    }
    unsafe fn draw(base: *mut NtAlgorithm) -> bool {
        (factory().draw.unwrap())(base)
    }
    unsafe fn custom_ui(base: *mut NtAlgorithm, event: *const NtUiData) {
        (factory().custom_ui.unwrap())(base, event)
    }
    unsafe fn setup_ui(base: *mut NtAlgorithm, pots: *mut NtFloat3) {
        (factory().setup_ui.unwrap())(base, pots)
    }
    unsafe fn serialise(base: *mut NtAlgorithm, stream: *mut NtJsonStream) {
        (factory().serialise.unwrap())(base, stream)
    }
    unsafe fn deserialise(base: *mut NtAlgorithm, parse: *mut NtJsonParse) -> bool {
        (factory().deserialise.unwrap())(base, parse)
    }
    unsafe fn parameter_string(
        base: *mut NtAlgorithm,
        p: i32,
        value: i32,
        output: *mut c_char,
    ) -> i32 {
        (factory().parameter_string.unwrap())(base, p, value, output)
    }
}
