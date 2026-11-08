//! Native host harness: exercise the exported factory rather than only DSP helpers.
extern crate std;
use super::*;
use core::{ffi::c_char, ptr};
use distingnt_sys::{
    NtAlgorithm, NtAlgorithmMemoryPtrs, NtAlgorithmRequirements, NtFactory, NtGlobals,
};
use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    cell::{Cell, RefCell},
    string::String,
    vec::Vec,
};

std::thread_local! {
    pub(super) static MIDI_TUNING_READS: Cell<usize> = const { Cell::new(0) };
    static SENT_SYSEX: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
    static CYCLE_COUNTER: Cell<u32> = const { Cell::new(0) };
    static FOOTER_COVER_AFTER_HELP: Cell<bool> = const { Cell::new(false) };
    static CURRENT: Cell<*mut NtAlgorithm> = const { Cell::new(ptr::null_mut()) };
    static CURRENT_SEGMENTS: Cell<usize> = const { Cell::new(0) };
    static GRAY_CALLS: RefCell<Vec<(usize, bool)>> = const { RefCell::new(Vec::new()) };
    static TEXT: RefCell<Vec<(i32, i32, String)>> = const { RefCell::new(Vec::new()) };
    static HEADER_COLORS: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static SHAPES: RefCell<Vec<(u8, i32, i32, i32, i32)>> = const { RefCell::new(Vec::new()) };
    static TRACE_LINES: RefCell<Vec<(f32, f32, f32, f32, f32)>> = const { RefCell::new(Vec::new()) };
    static GRID_COLORS: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static MARKER_COLORS: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
}

unsafe fn factory() -> &'static NtFactory {
    &*(pluginEntry(SELECTOR_FACTORY_INFO, 0) as *const NtFactory)
}

fn parameter_prefix(instance: &Instance, parameter: i32) -> String {
    let mut buffer = [0x55 as c_char; 16];
    let length = unsafe {
        factory().parameter_ui_prefix.unwrap()(instance.base, parameter, buffer.as_mut_ptr())
    };
    assert!((0..16).contains(&length));
    assert_eq!(buffer[length as usize], 0);
    unsafe { CStr::from_ptr(buffer.as_ptr()).to_str().unwrap().into() }
}

// Resolve editor controls against the live instance's segment count.
fn segment_parameter(segment: usize, field: usize) -> usize {
    let n = CURRENT_SEGMENTS.with(|current| current.get());
    editor_parameter(n, segment, field).expect("invalid editor control")
}

struct Instance {
    base: *mut NtAlgorithm,
    layout: Layout,
    guard_offset: usize,
    values: Vec<i16>,
}

impl Instance {
    fn new(n: i32) -> Self {
        Self::configured(n, 1)
    }
    fn with_voices(n: i32, voices: i32) -> Self {
        let mut instance = Self::configured(n, voices);
        // Explicitly spread groups for independent-note/stack regression tests.
        for voice in 0..voices as usize {
            instance.values[voice_midi_group_parameter(voice)] =
                (voice % midi::GROUP_COUNT) as i16 + 1;
        }
        instance
    }
    fn with_specified_voices(voices: i32) -> Self {
        Self::configured(DEFAULT_SEGMENTS as i32, voices)
    }
    fn configured(n: i32, specified: i32) -> Self {
        unsafe {
            let specs = [specified, n];
            let f = factory();
            let mut req = NtAlgorithmRequirements {
                num_parameters: 0,
                sram: 0,
                dram: 0,
                dtc: 0,
                itc: 0,
            };
            f.calculate_requirements.unwrap()(&mut req, specs.as_ptr());
            assert_eq!(
                req.num_parameters as usize,
                VOICE_START + specified as usize * VOICE_PARAMETERS
            );
            assert!(
                req.sram
                    < (dsp::blocks(dsp::DEFAULT_TABLES) * core::mem::size_of::<dsp::Block>()
                        + 14000) as u32
            );
            assert!(core::mem::size_of::<State>() < 8192);
            let alignment =
                distingnt_api::api::plugin::Registration::<Wavesmith, 2>::alignment(&specs)
                    .unwrap();
            let layout = Layout::from_size_align(req.sram as usize + 64, alignment).unwrap();
            let memory = alloc_zeroed(layout);
            assert!(!memory.is_null());
            ptr::write_bytes(memory.add(req.sram as usize), 0xa5, 64);
            let base = f.construct.unwrap()(
                &NtAlgorithmMemoryPtrs {
                    sram: memory,
                    dram: ptr::null_mut(),
                    dtc: ptr::null_mut(),
                    itc: ptr::null_mut(),
                },
                &req,
                specs.as_ptr(),
            );
            assert!(!base.is_null());
            let mut values: Vec<_> = (0..req.num_parameters as usize)
                .map(|i| (*(*base).parameters.add(i)).default)
                .collect();
            for boundary in 1..dsp::MAX_SEGMENTS {
                values[boundary_parameter(boundary)] = if boundary < n as usize {
                    dsp::default_level(boundary, n as usize)
                } else {
                    0
                };
            }
            (*base).v = values.as_ptr();
            CURRENT.with(|current| current.set(base));
            CURRENT_SEGMENTS.with(|current| current.set(n as usize));
            f.parameter_changed.unwrap()(base, curve_parameter(0) as i32);
            GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
            Self {
                base,
                layout,
                guard_offset: req.sram as usize,
                values,
            }
        }
    }
    fn change(&mut self, p: usize, value: i16) {
        self.values[p] = value;
        unsafe {
            factory().parameter_changed.unwrap()(self.base, p as i32);
        }
    }
    fn midi(&mut self, status: u8, data1: u8, data2: u8) {
        unsafe {
            factory().midi_message.unwrap()(self.base, status, data1, data2);
        }
    }
    fn render(&mut self, samples: &mut [f32]) {
        unsafe {
            factory().step.unwrap()(
                self.base,
                samples.as_mut_ptr(),
                (samples.len() / LAST_BUS / 4) as i32,
            );
        }
        self.assert_guard();
    }
    fn assert_guard(&self) {
        let guard = unsafe {
            core::slice::from_raw_parts(self.base.cast::<u8>().add(self.guard_offset), 64)
        };
        assert!(
            guard.iter().all(|&byte| byte == 0xa5),
            "algorithm wrote beyond its SRAM allocation"
        );
    }
    fn event(&mut self, pots: [f32; 3], encoders: [i8; 2]) {
        self.event_buttons(pots, encoders, 0, 0);
    }
    fn click_right(&mut self, pots: [f32; 3]) {
        self.event_buttons(pots, [0, 0], ENCODER_BUTTON_R, 0);
        self.event_buttons(pots, [0, 0], 0, ENCODER_BUTTON_R);
    }
    fn event_buttons(
        &mut self,
        pots: [f32; 3],
        encoders: [i8; 2],
        buttons: u32,
        last_buttons: u32,
    ) {
        let data = NtUiData {
            pots,
            encoders,
            controls: (POT_L | POT_C | POT_R | ENCODER_L | ENCODER_R | buttons) as u16,
            last_buttons: last_buttons as u16,
            unused: [0; 2],
        };
        unsafe {
            factory().custom_ui.unwrap()(self.base, &data);
        }
    }
    fn draw(&self) {
        TRACE_LINES.with(|lines| lines.borrow_mut().clear());
        GRID_COLORS.with(|colors| colors.borrow_mut().clear());
        MARKER_COLORS.with(|colors| colors.borrow_mut().clear());
        TEXT.with(|text| text.borrow_mut().clear());
        HEADER_COLORS.with(|colors| colors.borrow_mut().clear());
        SHAPES.with(|shapes| shapes.borrow_mut().clear());
        unsafe {
            assert!(factory().draw.unwrap()(self.base));
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.base.cast(), self.layout);
        }
        CURRENT.with(|current| current.set(ptr::null_mut()));
        CURRENT_SEGMENTS.with(|current| current.set(0));
    }
}

#[test]
fn every_segment_count_builds_complete_metadata_and_shared_seam() {
    for n in 2..=32 {
        let instance = Instance::configured(n, MAX_VOICES as i32);
        unsafe {
            assert_eq!(
                (*(*instance.base).parameter_pages).num_pages as usize,
                5 + MAX_VOICES
            );
            assert_eq!(editor_parameter(n as usize, 0, 0), Some(WAVEFORM_START));
            assert_eq!(
                editor_parameter(n as usize, n as usize - 1, 1),
                Some(WAVEFORM_START)
            );
            let wave = read_wave(n as usize, |p| instance.values.get(p).copied());
            assert_eq!(wave.segment_sample(0, 0.0), -1.0);
            assert_eq!(wave.segment_sample(n as usize - 1, 1.0), -1.0);
            let pages = &*(*instance.base).parameter_pages;
            let oscillator = &*pages.pages;
            assert_eq!(CStr::from_ptr(oscillator.name), c"Oscillator");
            assert_eq!(
                core::slice::from_raw_parts(oscillator.params, oscillator.num_params as usize),
                &[
                    MIDI_CHANNEL as u8,
                    PITCH as u8,
                    FINE as u8,
                    LEVEL as u8,
                    MIDI_ATTACK as u8,
                    MIDI_RELEASE as u8
                ]
            );
            for (index, page_name, expected_names) in [
                (
                    2,
                    c"Morph",
                    &[
                        c"Warp Position",
                        c"Warp Amount",
                        c"Smoothing",
                        c"Fold Limit",
                    ][..],
                ),
                (
                    3,
                    c"Reshape",
                    &[
                        c"Sine",
                        c"Triangle",
                        c"Square",
                        c"Sawtooth",
                        c"Zero Curves",
                        c"Randomize Curves",
                        c"Randomize Boundaries",
                        c"Randomize All",
                    ][..],
                ),
            ] {
                let page = &*pages.pages.add(index);
                assert_eq!(CStr::from_ptr(page.name), page_name);
                let rows = core::slice::from_raw_parts(page.params, page.num_params as usize);
                assert_eq!(rows.len(), expected_names.len());
                for (&index, name) in rows.iter().zip(expected_names) {
                    assert_eq!(
                        CStr::from_ptr((*(*instance.base).parameters.add(index as usize)).name),
                        *name
                    );
                }
            }
            let oscillator_group = (*pages.pages).group;
            let routing_group = (*pages.pages.add(1)).group;
            let page = &*pages.pages.add(MAX_VOICES + 4);
            assert_eq!(CStr::from_ptr(page.name), c"Waveform");
            assert_ne!(page.group, 0);
            assert_ne!(page.group, oscillator_group);
            assert_ne!(page.group, routing_group);
            let routing = &*pages.pages.add(1);
            assert_eq!(CStr::from_ptr(routing.name), c"Output");
            assert_eq!(
                core::slice::from_raw_parts(routing.params, routing.num_params as usize),
                &[LEFT_OUTPUT as u8, RIGHT_OUTPUT as u8, OUTPUT_MODE as u8]
            );
            assert_eq!(instance.values[LEFT_OUTPUT], 13);
            assert_eq!(instance.values[OUTPUT_MODE], 0);
            assert_eq!(instance.values[voice_pitch_parameter(0)], 1);
            assert_eq!(instance.values[PITCH], 48);
            assert_eq!(
                CStr::from_ptr((*(*instance.base).parameters.add(voice_pitch_parameter(0))).name),
                c"Pitch CV"
            );
            assert!(factory().parameter_ui_prefix.is_some());
            assert_eq!(page.num_params as usize, n as usize * 2 + 1);
            let rows = core::slice::from_raw_parts(page.params, page.num_params as usize);
            assert_eq!(rows[0] as usize, WAVEFORM_START);
            for segment in 0..n as usize {
                assert_eq!(rows[segment * 2 + 1] as usize, curve_parameter(segment));
                assert_eq!(
                    rows[segment * 2 + 2] as usize,
                    editor_parameter(n as usize, segment, 1).unwrap_or(WAVEFORM_START)
                );
            }
            let seam = *(*instance.base).parameters.add(WAVEFORM_START);
            assert_eq!((seam.min, seam.max, seam.default), (-100, 100, -100));
            assert_eq!(CStr::from_ptr(seam.name), c"Boundary 00");
            let mut references = std::vec![0; instance.values.len()];
            for i in 0..pages.num_pages as usize {
                let page = &*pages.pages.add(i);
                for j in 0..page.num_params as usize {
                    let p = *page.params.add(j) as usize;
                    assert!(p < instance.values.len());
                    references[p] += 1;
                }
            }
            for boundary in 1..n as usize {
                let p = boundary_parameter(boundary);
                assert_eq!(references[p], 1);
                assert_eq!(editor_parameter(n as usize, boundary - 1, 1), Some(p));
                assert_eq!(editor_parameter(n as usize, boundary, 0), Some(p));
                let d = *(*instance.base).parameters.add(p);
                assert_eq!((d.min, d.max), (-100, 100));
                assert_eq!(
                    CStr::from_ptr(d.name).to_str().unwrap(),
                    std::format!("Boundary {boundary:02}")
                );
            }
            for segment in 0..dsp::MAX_SEGMENTS {
                assert_eq!(
                    references[curve_parameter(segment)],
                    usize::from(segment < n as usize)
                );
            }
            assert_eq!(references[WAVEFORM_START], 2);
            assert_eq!(references[RANDOMIZE_ALL], 1);
            let midi = MIDI_CHANNEL;
            assert_eq!(references[midi], 1);
            assert_eq!(midi, 0);
            let d = *(*instance.base).parameters.add(midi);
            assert_eq!((d.min, d.max, d.default, d.unit), (0, 17, 0, UNIT_ENUM));
            assert_eq!(CStr::from_ptr(*d.enum_strings), c"Off");
            assert_eq!(CStr::from_ptr(*d.enum_strings.add(1)), c"All");
            assert_eq!(CStr::from_ptr(*d.enum_strings.add(17)), c"16");
            let fold = FOLD_LIMIT;
            assert_eq!(fold, 10);
            assert_eq!(references[fold], 1);
            let d = *(*instance.base).parameters.add(fold);
            assert_eq!(CStr::from_ptr(d.name), c"Fold Limit");
            assert_eq!((d.min, d.max, d.default, d.unit), (0, 2, 2, UNIT_NONE));
            for (index, name) in SHAPE_TRIGGERS.into_iter().zip([
                c"Randomize All",
                c"Randomize Boundaries",
                c"Randomize Curves",
                c"Zero Curves",
                c"Sine",
                c"Triangle",
                c"Square",
                c"Sawtooth",
            ]) {
                assert_eq!(references[index], 1);
                let d = *(*instance.base).parameters.add(index);
                assert_eq!(CStr::from_ptr(d.name), name);
                assert_eq!((d.min, d.max, d.default, d.unit), (0, 1, 0, UNIT_CONFIRM));
            }
            for (index, name, minimum, maximum, default) in [
                (WARP_POSITION, c"Warp Position", 0, 255, 128),
                (WARP_AMOUNT, c"Warp Amount", -100, 100, 0),
            ] {
                assert_eq!(references[index], 1);
                let d = *(*instance.base).parameters.add(index);
                assert_eq!(CStr::from_ptr(d.name), name);
                assert_eq!((d.min, d.max, d.default), (minimum, maximum, default));
            }
        }
    }
    for n in [0, 25, 33, -1] {
        assert!(Wavesmith::configuration(&[n, DEFAULT_SEGMENTS as i32]).is_none());
    }
    assert_eq!(pluginEntry(SELECTOR_VERSION, 0), API_VERSION_14);
}

#[test]
fn specification_selects_voices_while_live_segments_keep_six_tables() {
    assert_eq!(Wavesmith::SPECIFICATIONS.len(), 2);
    assert_eq!(Wavesmith::SPECIFICATIONS[0].name, c"Voices");
    assert_eq!(unsafe { factory() }.num_specifications, 2);
    for segments in 2..=32 {
        let mut instance = Instance::new(segments);
        assert_eq!(
            Wavesmith::configuration(&[MAX_VOICES as i32, segments])
                .unwrap()
                .regions,
            dsp::blocks(dsp::DEFAULT_TABLES)
        );
        let mut samples = [0.0; LAST_BUS * 24];
        instance.change(PITCH, 105);
        for smoothing in [0, 100] {
            instance.change(SMOOTHING, smoothing);
            for _ in 0..80 {
                instance.render(&mut samples);
            }
            assert!(samples.iter().all(|v| v.is_finite()));
            assert!(samples.iter().any(|v| v.abs() > 0.001));
            instance.draw();
        }
    }
}

#[test]
fn shape_triggers_change_only_the_requested_values_for_every_segment_count() {
    for n in 2..=32 {
        for kind in 0..4 {
            let mut instance = Instance::new(n);
            let n = n as usize;
            instance.change(SMOOTHING, 43);
            instance.change(WARP_POSITION, 94);
            instance.change(WARP_AMOUNT, -68);
            instance.change(FOLD_LIMIT, 1);
            let mut boundaries = std::vec![WAVEFORM_START];
            boundaries.extend((1..n).map(|boundary| boundary_parameter(boundary)));
            let curves: Vec<_> = (0..n).map(|segment| curve_parameter(segment)).collect();
            for (i, &parameter) in boundaries.iter().enumerate() {
                instance.change(parameter, (i as i16 * 3) - 40);
            }
            for (i, &parameter) in curves.iter().enumerate() {
                instance.change(parameter, 20 + i as i16);
            }
            let trigger = SHAPE_TRIGGERS[kind];
            let before = instance.values.clone();
            instance.change(trigger, 1);
            let after = instance.values.clone();
            for parameter in 0..before.len() {
                let changes_boundary = kind <= 1 && boundaries.contains(&parameter);
                let changes_curve = kind != 1 && curves.contains(&parameter);
                if parameter != trigger && !changes_boundary && !changes_curve {
                    assert_eq!(after[parameter], before[parameter]);
                }
            }
            if kind <= 1 {
                assert!(boundaries.iter().any(|&p| before[p] != after[p]));
            }
            if kind != 1 {
                assert!(curves.iter().any(|&p| before[p] != after[p]));
            }
            if kind == 3 {
                assert!(curves.iter().all(|&p| after[p] == 0));
            }
            assert_eq!(instance.values[trigger], 0);
            instance.change(trigger, 0);
            assert_eq!(instance.values, after);
            let rearmed = instance.values.clone();
            if kind == 3 {
                instance.change(curves[0], 73);
            }
            instance.change(trigger, 1);
            if kind == 3 {
                assert!(curves.iter().all(|&p| instance.values[p] == 0));
            } else {
                assert!(boundaries
                    .iter()
                    .chain(&curves)
                    .any(|&p| instance.values[p] != rearmed[p]));
            }
        }
    }
}

#[test]
fn reshape_actions_reset_independently() {
    let mut instance = Instance::new(8);
    let [all, boundaries, curves, zero, ..] = SHAPE_TRIGGERS;
    for trigger in [all, boundaries, curves, zero] {
        instance.change(trigger, 1);
    }
    let boundary_values: Vec<_> = (1..8)
        .map(|i| instance.values[boundary_parameter(i)])
        .collect();
    instance.change(curve_parameter(0), 73);
    let before = instance.values.clone();
    for trigger in [all, boundaries, curves, zero] {
        assert_eq!(instance.values[trigger], 0);
        instance.change(trigger, 0);
    }
    assert_eq!(instance.values, before);
    instance.change(curves, 0);
    instance.change(curves, 1);
    assert!((0..8).any(|i| instance.values[curve_parameter(i)] != before[curve_parameter(i)]));
    assert_eq!(
        (1..8)
            .map(|i| instance.values[boundary_parameter(i)])
            .collect::<Vec<_>>(),
        boundary_values
    );
    instance.change(zero, 0);
    instance.change(zero, 1);
    assert!((0..8).all(|i| instance.values[curve_parameter(i)] == 0));
}

#[test]
fn waveform_presets_zero_curves_preserve_globals_and_rearm() {
    for n in 2..=32 {
        let mut instance = Instance::new(n);
        let n = n as usize;
        instance.change(SMOOTHING, 43);
        instance.change(WARP_POSITION, 94);
        instance.change(WARP_AMOUNT, -68);
        for i in 0..n {
            instance.change(curve_parameter(i), 20 + i as i16);
        }
        let boundaries: Vec<_> = std::iter::once(WAVEFORM_START)
            .chain((1..n).map(|boundary| boundary_parameter(boundary)))
            .collect();
        for (kind, shape) in [
            dsp::BoundaryShape::Sine,
            dsp::BoundaryShape::Triangle,
            dsp::BoundaryShape::Square,
            dsp::BoundaryShape::Sawtooth,
        ]
        .into_iter()
        .enumerate()
        {
            for segment in 0..n {
                instance.change(curve_parameter(segment), 20 + segment as i16);
            }
            let trigger = SHAPE_TRIGGERS[4 + kind];
            let before = instance.values.clone();
            instance.change(trigger, 1);
            for (i, &parameter) in boundaries.iter().enumerate() {
                assert_eq!(
                    instance.values[parameter],
                    dsp::preset_boundary(shape, i, n)
                );
            }
            let curves: Vec<_> = (0..n).map(curve_parameter).collect();
            assert!(curves
                .iter()
                .all(|&parameter| instance.values[parameter] == 0));
            for parameter in 0..before.len() {
                if parameter != trigger
                    && !boundaries.contains(&parameter)
                    && !curves.contains(&parameter)
                {
                    assert_eq!(instance.values[parameter], before[parameter]);
                }
            }
            assert_eq!(instance.values[trigger], 0);
            instance.change(WAVEFORM_START, 73);
            // Automatic reset makes a second Apply work without manual rearming.
            instance.change(trigger, 1);
            assert_eq!(instance.values[trigger], 0);
            assert_eq!(instance.values[WAVEFORM_START], -100);
        }
    }
}

#[test]
fn randomize_actions_rearm_automatically_and_preserve_other_controls() {
    for n in 2..=32 {
        let mut instance = Instance::new(n);
        let n = n as usize;
        let trigger = RANDOMIZE_ALL;
        let shape_start = WAVEFORM_START;
        let shape_end = shape_start + n * 2;
        instance.change(SMOOTHING, 43);
        instance.change(WARP_POSITION, 37);
        instance.change(WARP_AMOUNT, -68);
        let before = instance.values.clone();
        instance.change(trigger, 1);
        let first = instance.values.clone();
        assert_eq!(first[WARP_POSITION], 37);
        assert_eq!(first[WARP_AMOUNT], -68);
        assert_ne!(
            &first[shape_start..shape_end],
            &before[shape_start..shape_end]
        );
        for index in 0..first.len() {
            if !(shape_start..shape_end).contains(&index) && index != trigger {
                assert_eq!(first[index], before[index]);
            }
        }
        assert!(first[shape_start..shape_end]
            .iter()
            .all(|v| (-100..=100).contains(v)));
        assert_eq!(instance.values[trigger], 0);
        instance.change(trigger, 0);
        assert_eq!(instance.values, first);
        instance.change(trigger, 1);
        assert_eq!(instance.values[trigger], 0);
        assert_ne!(
            &instance.values[shape_start..shape_end],
            &first[shape_start..shape_end]
        );
        let wave = read_wave(n, |p| instance.values.get(p).copied());
        let seam = instance.values[WAVEFORM_START] as f32 * 0.01;
        assert_eq!(wave.segment_sample(0, 0.0), seam);
        assert_eq!(wave.segment_sample(n - 1, 1.0), seam);
        for boundary in 1..n {
            assert_eq!(
                wave.segment_sample(boundary - 1, 1.0),
                wave.segment_sample(boundary, 0.0)
            );
        }
        let mut samples = [0.0; LAST_BUS * 64];
        for _ in 0..48 {
            instance.render(&mut samples);
        }
        assert!(samples.iter().all(|v| v.is_finite()));
        assert!(samples.iter().any(|v| v.abs() > 0.01));
    }
}

#[test]
fn both_end_pots_edit_the_same_seam_and_preview_follows_it() {
    for n in [2, 7, 32] {
        let mut instance = Instance::new(n);
        let n = n as usize;
        let seam = WAVEFORM_START;
        instance.change(seam, 25);
        instance.event([0.5; 3], [0, 0]);
        instance.event([0.508, 0.5, 0.5], [0, 0]);
        let after_left = instance.values[seam];
        assert!(after_left > 25);
        instance.event([0.508, 0.5, 0.5], [(n - 1) as i8, 0]);
        assert_eq!(segment_parameter(n - 1, 1), seam);
        instance.event([0.508, 0.5, 0.508], [0, 0]);
        assert!(instance.values[seam] > after_left);
        for level in [-100, 37, 100] {
            instance.change(seam, level);
            for smoothing in [0, 100] {
                instance.change(SMOOTHING, smoothing);
                let mut samples = [0.0; LAST_BUS * 64];
                for _ in 0..64 {
                    instance.render(&mut samples);
                }
                instance.draw();
                TRACE_LINES.with(|lines| {
                    let lines = lines.borrow();
                    assert_eq!(lines.first().unwrap().1, lines.last().unwrap().3);
                });
            }
        }
    }
}

#[test]
fn both_pots_edit_the_same_shared_boundary_from_neighboring_segments() {
    let mut instance = Instance::new(8);
    instance.event([0.5; 3], [2, 0]);
    let boundary = segment_parameter(2, 1);
    instance.change(boundary, 25);
    instance.event([0.5, 0.5, 0.508], [0, 0]);
    let after_right = instance.values[boundary];
    assert!(after_right > 25);

    instance.event([0.5, 0.5, 0.508], [1, 0]);
    assert_eq!(segment_parameter(3, 0), boundary);
    assert_eq!(instance.values[segment_parameter(3, 0)], after_right);
    assert_eq!(instance.values[segment_parameter(3, 0)], after_right);
    instance.event([0.508, 0.5, 0.508], [0, 0]);
    let after_left = instance.values[boundary];
    assert!(after_left > after_right);

    instance.event([0.508, 0.5, 0.508], [-1, 0]);
    assert_eq!(instance.values[segment_parameter(2, 1)], after_left);
    let wave = read_wave(8, |p| instance.values.get(p).copied());
    assert_eq!(wave.segment_sample(2, 1.0), wave.segment_sample(3, 0.0));

    // Direct host/preset/modulation writes use that same single source of truth.
    instance.change(boundary, -83);
    let wave = read_wave(8, |p| instance.values.get(p).copied());
    assert_eq!(wave.segment_sample(2, 1.0), -0.83);
    assert_eq!(wave.segment_sample(3, 0.0), -0.83);
}

#[test]
fn every_supported_layout_is_continuous_after_boundary_and_curve_edits() {
    for n in 2..=32 {
        let mut instance = Instance::new(n);
        for boundary in 1..n as usize {
            instance.change(
                boundary_parameter(boundary),
                (boundary * 37 % 201) as i16 - 100,
            );
        }
        for segment in 0..n as usize {
            instance.change(
                curve_parameter(segment),
                if segment % 2 == 0 { -100 } else { 100 },
            );
        }
        let wave = read_wave(n as usize, |p| instance.values.get(p).copied());
        for boundary in 1..n as usize {
            assert_eq!(
                wave.segment_sample(boundary - 1, 1.0),
                wave.segment_sample(boundary, 0.0)
            );
        }
        assert_eq!(wave.segment_sample(0, 0.0), -1.0);
        assert_eq!(wave.segment_sample(n as usize - 1, 1.0), -1.0);
    }
}

#[test]
fn fixed_twenty_four_voices_have_shared_routing_and_grouped_voice_pages() {
    assert_eq!(Wavesmith::SPECIFICATIONS.len(), 2);
    for n in 2..=32 {
        for voices in [MAX_VOICES] {
            let instance = Instance::configured(n, MAX_VOICES as i32);
            let config = Wavesmith::configuration(&[MAX_VOICES as i32, n]).unwrap();
            assert_eq!(config.pages, voices + 5);
            assert_eq!(
                config.regions * core::mem::size_of::<dsp::Block>(),
                24 * 1024
            );
            unsafe {
                let pages = &*(*instance.base).parameter_pages;
                let routing = &*pages.pages.add(1);
                let rows = core::slice::from_raw_parts(routing.params, routing.num_params as usize);
                assert_eq!(
                    rows,
                    &[LEFT_OUTPUT as u8, RIGHT_OUTPUT as u8, OUTPUT_MODE as u8]
                );
                let group = (*pages.pages.add(4)).group;
                assert_ne!(group, 0);
                assert_ne!(group, routing.group);
                assert_ne!(group, (*pages.pages).group);
                assert_ne!(group, (*pages.pages.add(voices + 4)).group);
                let mut seen = std::vec![0; instance.values.len()];
                let mut next_ordinal = 0;
                for i in 0..pages.num_pages as usize {
                    let page = &*pages.pages.add(i);
                    for &p in core::slice::from_raw_parts(page.params, page.num_params as usize) {
                        if seen[p as usize] == 0 {
                            next_ordinal += 1;
                        }
                        seen[p as usize] += 1;
                    }
                }
                assert_eq!(
                    next_ordinal,
                    GLOBALS + 2 + MAX_VOICES * VOICE_PARAMETERS + n as usize * 2
                );
                for (p, references) in seen.into_iter().enumerate() {
                    assert_eq!(
                        references,
                        if p == WAVEFORM_START {
                            2
                        } else if (WAVEFORM_START..MIDI_ATTACK).contains(&p)
                            && (p - WAVEFORM_START) / 2 >= n as usize
                        {
                            0
                        } else {
                            1
                        }
                    );
                }
                for voice in 0..voices {
                    let page = &*pages.pages.add(voice + 4);
                    assert_eq!(
                        CStr::from_ptr(page.name).to_str().unwrap(),
                        std::format!("Voice {}", voice + 1)
                    );
                    assert_eq!(page.group, group);
                    let cv = voice_pitch_parameter(voice);
                    let transpose = voice_transpose_parameter(voice);
                    let fine = voice_fine_parameter(voice);
                    assert_eq!(
                        core::slice::from_raw_parts(page.params, page.num_params as usize),
                        &[
                            cv as u8,
                            transpose as u8,
                            fine as u8,
                            voice_pan_parameter(voice) as u8,
                            voice_midi_group_parameter(voice) as u8
                        ]
                    );
                    let pan = voice_pan_parameter(voice);
                    let d = &*(*instance.base).parameters.add(pan);
                    assert_eq!(CStr::from_ptr(d.name), c"Pan");
                    assert_eq!(
                        (d.min, d.max, d.default, d.unit),
                        (0, 100, 50, UNIT_PERCENT)
                    );
                    assert_eq!(
                        parameter_prefix(&instance, pan as i32),
                        std::format!("V{}:", voice + 1)
                    );
                    let group = voice_midi_group_parameter(voice);
                    let d = &*(*instance.base).parameters.add(group);
                    assert_eq!(CStr::from_ptr(d.name), c"MIDI Voice Group");
                    assert_eq!(
                        (d.min, d.max, d.default, d.unit),
                        (0, 10, default_group_value(voice), UNIT_ENUM)
                    );
                    for (i, label) in [
                        c"None", c"1", c"2", c"3", c"4", c"5", c"6", c"7", c"8", c"9", c"10",
                    ]
                    .iter()
                    .enumerate()
                    {
                        assert_eq!(CStr::from_ptr(*d.enum_strings.add(i)), *label);
                    }
                    for (index, name, min, max, default, unit) in [
                        (cv, c"Pitch CV", 0, LAST_BUS as i16, 1, UNIT_CV_INPUT),
                        (transpose, c"Transpose", -48, 48, 0, UNIT_SEMITONES),
                        (fine, c"Fine tune", -100, 100, 0, UNIT_CENTS),
                    ] {
                        let d = &*(*instance.base).parameters.add(index);
                        assert_eq!(CStr::from_ptr(d.name), name);
                        assert_eq!((d.min, d.max, d.default, d.unit), (min, max, default, unit));
                        assert_eq!(
                            parameter_prefix(&instance, index as i32),
                            std::format!("V{}:", voice + 1)
                        );
                    }
                }
            }
        }
    }
    assert!(Wavesmith::configuration(&[]).is_none());
    assert!(Wavesmith::configuration(&[8, 8]).is_some());
}

#[test]
fn stable_midi_and_cv_reuse_caches_for_only_the_configured_pool() {
    const FRAMES: usize = 24;
    for voices in 1..=MAX_VOICES {
        let mut instance = Instance::with_voices(8, voices as i32);
        instance.change(OUTPUT_MODE, 1);
        for voice in 0..voices {
            instance.change(
                voice_pitch_parameter(voice),
                if voice < 12 {
                    voice as i16 + 1
                } else {
                    voice as i16 + 2
                },
            );
        }
        instance.change(MIDI_CHANNEL, 1);
        for voice in 0..voices {
            instance.midi(0x90, 60 + voice as u8, 100);
        }
        let mut samples = [0.0; LAST_BUS * FRAMES];
        for _ in 0..160 {
            instance.render(&mut samples);
        }
        let before = dsp::playback_work_counts();
        for _ in 0..100 {
            instance.render(&mut samples);
        }
        let after = dsp::playback_work_counts();
        assert_eq!(after.0 - before.0, 100 * FRAMES * voices);
        assert_eq!(after.1, before.1);
        instance.change(FINE, 10);
        instance.render(&mut samples);
        assert_eq!(dsp::playback_work_counts().1 - after.1, voices);
        instance.change(MIDI_CHANNEL, 0);
        for _ in 0..160 {
            instance.render(&mut samples);
        }
        let before = dsp::playback_work_counts();
        for _ in 0..100 {
            instance.render(&mut samples);
        }
        let after = dsp::playback_work_counts();
        assert_eq!(after.0 - before.0, 100 * FRAMES * voices);
        assert_eq!(after.1, before.1);
        samples[..FRAMES].fill(0.00001);
        instance.render(&mut samples);
        assert_eq!(dsp::playback_work_counts().1 - after.1, 1);
    }
}

#[test]
fn midi_gates_the_specified_pool_ignores_cv_and_restores_cv_when_off() {
    const FRAMES: usize = 24;
    for voices in 1..=MAX_VOICES {
        let mut instance = Instance::with_voices(8, voices as i32);
        instance.change(MIDI_RELEASE, 25); // Explicit timing fixture, independent of defaults.
        instance.change(OUTPUT_MODE, 1);
        instance.change(MIDI_CHANNEL, 2);
        instance.change(PITCH, 0);
        for voice in 0..voices {
            instance.change(voice_pitch_parameter(voice), voice as i16 + 1);
        }
        let mut samples = [0.0; LAST_BUS * FRAMES];
        let before = dsp::oscillator_tick_count();
        for _ in 0..60 {
            instance.render(&mut samples);
        }
        instance.midi(0x91, 60, 100);
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count(), before);
        for voice in 0..voices {
            instance.midi(0x90, 60 + voice as u8, 100);
        }
        for _ in 0..160 {
            for voice in 0..voices {
                samples[voice * FRAMES..(voice + 1) * FRAMES].fill(8.0);
            }
            instance.render(&mut samples);
        }
        let before = dsp::playback_work_counts();
        instance.change(PITCH, 127);
        for voice in 0..voices {
            samples[voice * FRAMES..(voice + 1) * FRAMES].fill(-8.0);
        }
        instance.render(&mut samples);
        assert_eq!(dsp::playback_work_counts().0 - before.0, voices * FRAMES);
        assert_eq!(dsp::playback_work_counts().1, before.1);
        instance.midi(0x90, 84, 100); // Steal oldest.
        instance.midi(0x80, 60, 0); // Stale release must not stop replacement.
        for _ in 0..30 {
            instance.render(&mut samples);
        }
        let before = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count() - before, voices * FRAMES);
        for voice in 1..voices {
            instance.midi(0x90, 60 + voice as u8, 0);
        }
        instance.midi(0x90, 84, 0);
        for _ in 0..60 {
            instance.render(&mut samples);
        }
        let before = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count(), before);
        assert!(samples[12 * FRAMES..13 * FRAMES].iter().all(|&v| v == 0.0));
        instance.change(MIDI_CHANNEL, 0);
        instance.change(PITCH, 57);
        samples.fill(0.0);
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count() - before, voices * FRAMES);
    }
}

#[test]
fn shared_output_none_skips_all_voices_and_clears_midi_notes() {
    for voices in 1..=MAX_VOICES {
        let mut instance = Instance::with_voices(8, voices as i32);
        instance.change(OUTPUT_MODE, 1);
        instance.change(MIDI_CHANNEL, 1);
        for voice in 0..voices {
            instance.midi(0x90, 60 + voice as u8, 100);
        }
        let mut samples = [0.0; LAST_BUS * 24];
        for _ in 0..80 {
            instance.render(&mut samples);
        }
        instance.change(LEFT_OUTPUT, 0);
        let before = dsp::oscillator_tick_count();
        samples.fill(2.0);
        for _ in 0..80 {
            instance.render(&mut samples);
        }
        assert_eq!(dsp::oscillator_tick_count(), before);
        assert!(samples.iter().all(|&v| v == 2.0));
        instance.change(LEFT_OUTPUT, 13);
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count(), before);
        assert!(samples[12 * 24..13 * 24].iter().all(|&v| v == 0.0));
        instance.midi(0x90, 67, 100);
        instance.render(&mut samples);
        assert_eq!(
            dsp::oscillator_tick_count() - before,
            voices.div_ceil(midi::GROUP_COUNT) * 24
        );
    }
}

#[test]
fn shared_cv_output_normalizes_sum_and_snapshots_inputs_first() {
    const FRAMES: usize = 24;
    for voices in 1..=MAX_VOICES {
        let mut mixed = Instance::with_voices(8, voices as i32);
        mixed.change(LEFT_OUTPUT, 1); // Overwrites Voice 1's pitch input after snapshot.
        mixed.change(OUTPUT_MODE, 1);
        for voice in 0..voices {
            mixed.change(voice_pitch_parameter(voice), voice as i16 + 1);
        }
        let mut control = levels::LevelControl::new(48000);
        let mut references: Vec<_> = (0..voices)
            .map(|_| {
                let mut mono = Instance::new(8);
                mono.change(OUTPUT_MODE, 1);
                mono.change(voice_pitch_parameter(0), 1);
                mono
            })
            .collect();
        let mut samples = [0.0; LAST_BUS * FRAMES];
        for _ in 0..120 {
            for voice in 0..voices {
                samples[voice * FRAMES..(voice + 1) * FRAMES].fill(voice as f32 * 0.25);
            }
            mixed.render(&mut samples);
            let mut expected = [0.0; FRAMES];
            let mut powers = [0.0; FRAMES];
            for (voice, mono) in references.iter_mut().enumerate() {
                let mut buffer = [0.0; LAST_BUS * FRAMES];
                buffer[..FRAMES].fill(voice as f32 * 0.25);
                mono.render(&mut buffer);
                for ((sum, power), &v) in expected
                    .iter_mut()
                    .zip(powers.iter_mut())
                    .zip(&buffer[12 * FRAMES..13 * FRAMES])
                {
                    *sum += v;
                    *power += v * v;
                }
            }
            for (chunk, powers) in expected
                .chunks_mut(levels::CHUNK)
                .zip(powers.chunks(levels::CHUNK))
            {
                control.process(
                    chunk,
                    powers.iter().sum(),
                    voices as f32,
                    levels::PEAK_CEILING,
                );
            }
            for (&actual, &expected) in samples[..FRAMES].iter().zip(&expected) {
                assert!(
                    (actual - expected).abs() < 0.00001,
                    "voices {voices}: {actual} vs {expected}"
                );
            }
        }
        let before = dsp::oscillator_tick_count();
        mixed.change(LEFT_OUTPUT, 0);
        mixed.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count(), before);
    }
}

#[test]
fn configured_unison_voices_settle_near_mono_level_and_replace_preserves_other_buses() {
    let mut reference: Option<Vec<f32>> = None;
    for voices in 1..=MAX_VOICES {
        let mut instance = Instance::with_voices(8, voices as i32);
        instance.change(OUTPUT_MODE, 1);
        let mut samples = [0.0; LAST_BUS * 64];
        for _ in 0..1500 {
            samples.fill(2.0);
            instance.render(&mut samples);
        }
        assert!(samples[..12 * 64].iter().all(|&v| v == 2.0));
        assert!(samples[13 * 64..].iter().all(|&v| v == 2.0));
        let output = samples[12 * 64..13 * 64].to_vec();
        assert!(output
            .iter()
            .all(|v| v.abs() <= levels::PEAK_CEILING + 0.000001));
        if let Some(mono) = &reference {
            for (&actual, &v) in output.iter().zip(mono) {
                assert!((actual - v).abs() < 0.003);
            }
        } else {
            reference = Some(output);
        }
    }
}

#[test]
fn output_modes_routing_pitch_cv_and_live_shape_edits() {
    let mut instance = Instance::new(8);
    instance.change(LEFT_OUTPUT, 64);
    instance.change(OUTPUT_MODE, 1);
    let mut samples = [0.0; LAST_BUS * 64];
    for _ in 0..64 {
        instance.render(&mut samples);
    }
    assert!(samples[..63 * 64].iter().all(|&v| v == 0.0));
    assert!(samples[63 * 64..].iter().any(|v| v.abs() > 0.1));
    instance.change(voice_pitch_parameter(0), 1);
    samples[..64].fill(1.0);
    instance.render(&mut samples);
    let before = samples[63 * 64..].to_vec();
    instance.change(segment_parameter(2, 1), -100);
    assert_eq!(segment_parameter(2, 1), segment_parameter(3, 0));
    instance.change(segment_parameter(3, 2), 80);
    instance.change(SMOOTHING, 80);
    for _ in 0..64 {
        instance.render(&mut samples);
    }
    assert_ne!(before, samples[63 * 64..]);
    assert!(samples[63 * 64..]
        .iter()
        .all(|v| v.is_finite() && v.abs() < 5.0));
    instance.change(LEVEL, levels::LEVEL_MIN);
    for _ in 0..100 {
        instance.render(&mut samples);
    }
    instance.change(OUTPUT_MODE, 0);
    samples[63 * 64..].fill(2.0);
    instance.render(&mut samples);
    assert!(samples[63 * 64..]
        .iter()
        .all(|&v| (v - 2.0).abs() < 0.00001));
}

#[test]
fn only_bright_editor_trace_is_drawn_with_or_without_smoothing() {
    let mut instance = Instance::new(7);
    instance.change(curve_parameter(2), 100);
    let mut samples = [0.0; LAST_BUS * 64];
    let mut original = None;
    for smoothing in [0, 75, 0] {
        instance.change(SMOOTHING, smoothing);
        for _ in 0..64 {
            instance.render(&mut samples);
        }
        instance.draw();
        TRACE_LINES.with(|lines| {
            let lines = lines.borrow();
            assert!(lines.len() >= 255 && lines.len() <= 255 + 32);
            assert!(lines.iter().all(|line| line.4 == 15.0));
            assert!(lines
                .iter()
                .any(|line| line.1 != line.1 as i32 as f32 || line.3 != line.3 as i32 as f32));
            if let Some(ref original) = original {
                assert_eq!(*original == *lines, smoothing == 0);
            } else {
                original = Some(lines.clone());
            }
        });
        GRID_COLORS.with(|colors| {
            // Six dotted dividers and the solid axis stay at minimum brightness.
            assert_eq!(colors.borrow().len(), 61);
            assert!(colors.borrow().iter().all(|&color| color == 1));
        });
        MARKER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[15, 15]));
    }
}

#[test]
fn pitch_changes_do_not_change_the_displayed_waveform() {
    let mut instance = Instance::new(32);
    instance.change(curve_parameter(0), 100);
    let mut samples = [0.0; LAST_BUS * 64];
    for _ in 0..64 {
        instance.render(&mut samples);
    }
    instance.draw();
    let low = TRACE_LINES.with(|lines| lines.borrow().clone());
    instance.change(PITCH, 105);
    for _ in 0..64 {
        instance.render(&mut samples);
    }
    instance.draw();
    TRACE_LINES.with(|lines| {
        let lines = lines.borrow();
        assert_eq!(*lines, low);
        assert!(lines.iter().all(|line| line.4 == 15.0));
    });
}

#[test]
fn drawn_endpoint_handles_do_not_move_with_curve_or_smoothing() {
    let mut instance = Instance::new(8);
    instance.event([0.5; 3], [1, 0]);
    instance.change(segment_parameter(1, 0), 75);
    instance.change(segment_parameter(1, 1), -25);
    for curve in [-100, 0, 100] {
        for smoothing in [0, 100] {
            instance.change(segment_parameter(1, 2), curve);
            instance.change(SMOOTHING, smoothing);
            let mut samples = [0.0; LAST_BUS * 64];
            for _ in 0..64 {
                instance.render(&mut samples);
            }
            instance.draw();
            SHAPES.with(|shapes| {
                let shapes = shapes.borrow();
                // Segment 2 spans x=31..63; +75% maps to y=18 and -25% to y=31.
                assert!(shapes.contains(&(SHAPE_BOX, 30, 17, 32, 19)));
                assert!(shapes.contains(&(SHAPE_BOX, 62, 30, 64, 32)));
            });
            TRACE_LINES.with(|lines| {
                assert!(lines
                    .borrow()
                    .iter()
                    .any(|line| line.2 == 63.75 && (line.3 - 31.25).abs() < 0.001))
            });
        }
    }
}

#[test]
fn smart_pickup_reaches_both_stops_for_every_pot_after_segment_selection() {
    let mut instance = Instance::new(8);
    for field in 0..3 {
        instance.change(segment_parameter(1, field), -80);
    }
    instance.event([0.8; 3], [1, 0]);
    for field in 0..3 {
        assert_eq!(instance.values[segment_parameter(1, field)], -80);
    }
    for step in 1..=40 {
        instance.event([0.8 + step as f32 * 0.005; 3], [0, 0]);
    }
    for field in 0..3 {
        assert_eq!(instance.values[segment_parameter(1, field)], 100);
    }
    instance.event([0.995; 3], [0, 0]);
    for field in 0..3 {
        assert!(instance.values[segment_parameter(1, field)] < 100);
    }
    for step in (0..=198).rev() {
        instance.event([step as f32 * 0.005; 3], [0, 0]);
    }
    for field in 0..3 {
        assert_eq!(instance.values[segment_parameter(1, field)], -100);
    }
    instance.event([0.005; 3], [0, 0]);
    for field in 0..3 {
        assert!(instance.values[segment_parameter(1, field)] > -100);
    }

    // Another screen moved the pots: ignore its jump, then resume from there.
    instance.event([0.5; 3], [0, 0]);
    for field in 0..3 {
        instance.change(segment_parameter(1, field), -50);
    }
    for step in 1..=100 {
        instance.event([0.5 + step as f32 * 0.0001; 3], [0, 0]);
    }
    for field in 0..3 {
        let value = instance.values[segment_parameter(1, field)];
        assert!(
            value > -50 && value < -45,
            "slow movement lost or jumped: {value}"
        );
    }
}

#[test]
fn warp_position_marker_grid_and_handles_follow_redistributed_boundaries() {
    let mut instance = Instance::new(8);
    instance.event([0.5; 3], [2, 0]);
    instance.change(WARP_POSITION, 94);
    for warp_amount in [-100, 100] {
        instance.change(WARP_AMOUNT, warp_amount);
        instance.change(SMOOTHING, 50);
        let wave = read_wave(8, |p| instance.values.get(p).copied());
        let left = (wave.boundary_position(2) * 255.0) as i32;
        let right = (wave.boundary_position(3) * 255.0) as i32;
        let mut samples = [0.0; LAST_BUS * 64];
        for _ in 0..48 {
            instance.render(&mut samples);
        }
        instance.draw();
        SHAPES.with(|shapes| {
            let shapes = shapes.borrow();
            assert!(shapes.contains(&(SHAPE_RECTANGLE, left, 14, right, 42)));
            assert!(!shapes.contains(&(SHAPE_RECTANGLE, 94, 14, 95, 42)));
            assert!(!shapes.contains(&(SHAPE_RECTANGLE, 93, 14, 96, 42)));
            for i in 1..8 {
                let x = (wave.boundary_position(i) * 255.0) as i32;
                for y in (14..=42).step_by(3) {
                    assert!(shapes.contains(&(SHAPE_POINT, x, y, x, y)));
                }
                assert!(!shapes.contains(&(SHAPE_LINE, x, 14, x, 42)));
            }
        });
        TRACE_LINES.with(|lines| {
            let lines = lines.borrow();
            let bright = &*lines;
            for i in 1..8 {
                let x = wave.boundary_position(i) * 255.0;
                let y = (28.0 - wave.boundary_level(i) * 13.0).clamp(14.0, 42.0);
                assert!(bright
                    .iter()
                    .any(|line| line.2 == x && (line.3 - y).abs() < 0.01));
            }
        });
        instance.click_right([0.5; 3]);
        instance.draw();
        SHAPES.with(|shapes| {
            let shapes = shapes.borrow();
            assert!(shapes.contains(&(SHAPE_RECTANGLE, 94, 14, 95, 42)));
            assert!(shapes.contains(&(SHAPE_RECTANGLE, 93, 14, 96, 42)));
            assert!(!shapes.iter().any(|shape| shape.0 == SHAPE_BOX));
        });
        instance.click_right([0.5; 3]);
        instance.draw();
        SHAPES.with(|shapes| {
            assert!(!shapes
                .borrow()
                .iter()
                .any(|shape| shape.0 == SHAPE_RECTANGLE && shape.2 == 14));
        });
        instance.click_right([0.5; 3]);
    }
}

#[test]
fn tuning_mode_edits_pitch_fine_and_level_with_pickup_and_wraps_back_to_segment() {
    let mut instance = Instance::new(8);
    instance.event([0.8; 3], [2, 0]);
    instance.change(PITCH, 57);
    instance.change(FINE, -25);
    instance.change(LEVEL, -9);
    instance.change(WARP_POSITION, 94);
    instance.change(WARP_AMOUNT, -68);
    instance.change(SMOOTHING, 43);
    let before = instance.values.clone();
    instance.click_right([0.8; 3]);
    instance.click_right([0.8; 3]);
    assert_eq!(instance.values, before);
    instance.draw();
    TEXT.with(|text| {
        let text = text.borrow();
        for expected in ["Pitch A3", "Fine -25", "Level -9 dB"] {
            assert!(text
                .iter()
                .any(|(_, y, value)| *y == 10 && value == expected));
        }
        let footer: Vec<_> = text
            .iter()
            .filter(|(_, y, _)| *y == 64)
            .map(|(_, _, value)| value.as_str())
            .collect();
        assert_eq!(footer, ["Push: Mode"]);
    });
    HEADER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[15; 3]));
    SHAPES.with(|shapes| {
        let shapes = shapes.borrow();
        assert!(shapes.contains(&(SHAPE_RECTANGLE, 0, 0, 255, 12)));
        assert!(!shapes.iter().any(|shape| shape.0 == SHAPE_BOX));
        assert!(!shapes
            .iter()
            .any(|shape| shape.0 == SHAPE_RECTANGLE && shape.2 == 14));
    });
    for step in 1..=40 {
        instance.event([0.8 + step as f32 * 0.005; 3], [0, 0]);
    }
    assert_eq!(
        (
            instance.values[PITCH],
            instance.values[FINE],
            instance.values[LEVEL]
        ),
        (127, 100, 0)
    );
    for step in (0..200).rev() {
        instance.event([step as f32 * 0.005; 3], [0, 0]);
    }
    assert_eq!(
        (
            instance.values[PITCH],
            instance.values[FINE],
            instance.values[LEVEL]
        ),
        (0, -100, levels::LEVEL_MIN)
    );
    for step in 1..=100 {
        instance.event([step as f32 * 0.0002; 3], [0, 0]);
    }
    assert!(
        instance.values[PITCH] > 0
            && instance.values[FINE] > -100
            && instance.values[LEVEL] > levels::LEVEL_MIN
    );
    for parameter in 0..before.len() {
        if ![PITCH, FINE, LEVEL].contains(&parameter) {
            assert_eq!(instance.values[parameter], before[parameter]);
        }
    }
    let before_switch = instance.values.clone();
    instance.click_right([0.5; 3]);
    assert_eq!(instance.values, before_switch);
    instance.draw();
    HEADER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[15; 3]));
    TEXT.with(|text| {
        assert!(text
            .borrow()
            .iter()
            .any(|(_, _, value)| value == "Push: Mode"))
    });
    SHAPES.with(|shapes| {
        assert_eq!(
            shapes
                .borrow()
                .iter()
                .filter(|shape| shape.0 == SHAPE_BOX)
                .count(),
            2
        )
    });
}

#[test]
fn segment_encoder_wraps_both_directions_and_large_deltas_without_pot_jumps() {
    for n in 2..=32 {
        let mut instance = Instance::new(n);
        let n = n as usize;
        for segment in 0..n {
            instance.change(curve_parameter(segment), segment as i16 * 3 - 50);
        }
        instance.event([0.5; 3], [0, 0]);
        let before = instance.values.clone();
        let mut selected = 0i32;
        for delta in [-1i8, 1, 127, -128, 1, -1] {
            selected = (selected + delta as i32).rem_euclid(n as i32);
            instance.event([0.508; 3], [delta, 0]);
            assert_eq!(instance.values, before);
            instance.draw();
            let expected = std::format!("Curve {}", before[curve_parameter(selected as usize)]);
            TEXT.with(|text| {
                assert!(text
                    .borrow()
                    .iter()
                    .any(|(_, y, value)| *y == 10 && value == &expected))
            });
        }
        instance.event([0.508, 0.517, 0.508], [0, 0]);
        let edited = curve_parameter(selected as usize);
        assert!(instance.values[edited] > before[edited]);
        for p in 0..before.len() {
            if p != edited {
                assert_eq!(instance.values[p], before[p]);
            }
        }
    }
}

#[test]
fn segment_header_shows_selected_left_curve_and_right_values_with_background() {
    for n in [2, 7, 32] {
        let mut instance = Instance::new(n);
        let n = n as usize;
        let mut previous = 0;
        for selected in [0, n / 2, n - 1] {
            instance.event([0.5; 3], [(selected - previous) as i8, 0]);
            previous = selected;
            instance.change(editor_parameter(n, selected, 0).unwrap(), -100);
            instance.change(curve_parameter(selected), -37);
            instance.change(editor_parameter(n, selected, 1).unwrap(), 100);
            instance.draw();
            TEXT.with(|text| {
                for (x, expected) in [(2, "Left -100"), (102, "Curve -37"), (190, "Right 100")] {
                    assert!(text
                        .borrow()
                        .iter()
                        .any(|(tx, y, value)| *tx == x && *y == 10 && value == expected));
                }
            });
            SHAPES.with(|shapes| {
                assert!(shapes.borrow().contains(&(SHAPE_RECTANGLE, 0, 0, 255, 12)))
            });
            HEADER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[15; 3]));
        }
    }
}

#[test]
fn tuning_pitch_header_uses_nt_note_names_and_octaves() {
    let mut instance = Instance::new(8);
    instance.event([0.5; 3], [0, 0]);
    instance.click_right([0.5; 3]);
    instance.click_right([0.5; 3]);
    for (note, expected) in [
        (0, "Pitch C-1"),
        (1, "Pitch C#-1"),
        (11, "Pitch B-1"),
        (12, "Pitch C0"),
        (36, "Pitch C2"),
        (37, "Pitch C#2"),
        (38, "Pitch D2"),
        (39, "Pitch D#2"),
        (40, "Pitch E2"),
        (41, "Pitch F2"),
        (42, "Pitch F#2"),
        (43, "Pitch G2"),
        (44, "Pitch G#2"),
        (45, "Pitch A2"),
        (46, "Pitch A#2"),
        (47, "Pitch B2"),
        (48, "Pitch C3"),
        (57, "Pitch A3"),
        (69, "Pitch A4"),
        (127, "Pitch G9"),
    ] {
        instance.change(PITCH, note);
        instance.draw();
        TEXT.with(|text| {
            assert!(text
                .borrow()
                .iter()
                .any(|(x, y, value)| *x == 2 && *y == 10 && value == expected))
        });
        assert_eq!(instance.values[PITCH], note);
    }
}

#[test]
fn right_encoder_click_switches_pot_modes_without_jumps_and_reaches_all_ranges() {
    let mut instance = Instance::new(8);
    instance.event([0.8; 3], [2, 0]);
    instance.change(SMOOTHING, 25);
    instance.change(LEVEL, -10);
    instance.change(WARP_POSITION, 25);
    instance.change(WARP_AMOUNT, 0);
    instance.change(PITCH, 40);
    let before = instance.values.clone();
    instance.event_buttons([0.8; 3], [0, 0], ENCODER_BUTTON_R, 0);
    assert_eq!(instance.values, before);
    // Holding the button must not switch back; releasing must not switch either.
    instance.event_buttons([0.8; 3], [0, 0], ENCODER_BUTTON_R, ENCODER_BUTTON_R);
    instance.event_buttons([0.8; 3], [0, 0], 0, ENCODER_BUTTON_R);
    instance.draw();
    SHAPES.with(|shapes| {
        let shapes = shapes.borrow();
        assert!(!shapes.contains(&(SHAPE_RECTANGLE, 0, 0, 255, 7)));
        assert!(shapes.contains(&(SHAPE_RECTANGLE, 0, 0, 255, 12)));
        assert!(!shapes.iter().any(|s| s.0 == SHAPE_BOX));
        assert_eq!(
            shapes
                .iter()
                .filter(|s| s.0 == SHAPE_RECTANGLE && s.2 == 14)
                .count(),
            2
        );
    });
    HEADER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[15, 15, 15]));
    TRACE_LINES.with(|lines| {
        let lines = lines.borrow();
        assert!(lines.len() >= 255 && lines.len() <= 255 + 32);
        assert!(lines.iter().all(|line| line.4 == 15.0));
    });
    GRID_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[1; 71]));
    SHAPES.with(|shapes| {
        let shapes = shapes.borrow();
        assert!(shapes.contains(&(SHAPE_LINE, 0, 28, 255, 28)));
        assert_eq!(
            shapes
                .iter()
                .filter(|s| s.0 == SHAPE_POINT && s.2 >= 14 && s.4 <= 42)
                .count(),
            70
        );
    });
    TEXT.with(|text| {
        let text = text.borrow();
        let footer: Vec<_> = text
            .iter()
            .filter(|(_, y, _)| *y == 64)
            .map(|(_, _, value)| value.as_str())
            .collect();
        assert_eq!(footer, ["Push: Mode"]);
    });
    for step in 1..=40 {
        instance.event([0.8 + step as f32 * 0.005; 3], [0, 0]);
    }
    assert_eq!(instance.values[SMOOTHING], 100);
    assert_eq!(instance.values[WARP_AMOUNT], 100);
    let trigger = RANDOMIZE_ALL;
    assert_eq!(instance.values[trigger], 0);
    assert_eq!(instance.values[WARP_POSITION], 255);
    assert_eq!(instance.values[LEVEL], -10);
    assert_eq!(instance.values[PITCH], 40);
    let first_shape = instance.values[WAVEFORM_START..].to_vec();
    assert_eq!(first_shape, before[WAVEFORM_START..]);
    instance.event([1.0; 3], [0, 0]);
    assert_eq!(&instance.values[WAVEFORM_START..], &first_shape);
    for step in (0..200).rev() {
        instance.event([step as f32 * 0.005; 3], [0, 0]);
    }
    assert_eq!(instance.values[SMOOTHING], 0);
    assert_eq!(instance.values[WARP_AMOUNT], -100);
    assert_eq!(instance.values[trigger], 0);
    assert_eq!(instance.values[WARP_POSITION], 0);
    assert_eq!(&instance.values[WAVEFORM_START..], &first_shape);
    assert_eq!(instance.values[PITCH], 40);
    // Slow motion must accumulate instead of disappearing in integer rounding.
    for step in 1..=100 {
        instance.event([step as f32 * 0.0002; 3], [0, 0]);
    }
    assert!(instance.values[SMOOTHING] > 0);
    assert!(instance.values[WARP_AMOUNT] > -100);
    assert!(instance.values[WARP_POSITION] > 0);
    assert_eq!(instance.values[PITCH], 40);
    instance.draw();
    TEXT.with(|text| {
        assert!(text.borrow().iter().any(|(x, y, s)| *x == 198
            && *y == 10
            && s == &std::format!("Smooth {}", instance.values[SMOOTHING])));
    });
    // Changing modes alongside pot movement re-primes pickup without edits.
    instance.change(WARP_AMOUNT, 0);
    // Visit Tuning, then click again to return to Segment mode.
    instance.event_buttons([0.5; 3], [0, 0], ENCODER_BUTTON_R, 0);
    instance.event_buttons([0.5; 3], [0, 0], 0, ENCODER_BUTTON_R);
    let before_switch = instance.values.clone();
    instance.event_buttons([0.5; 3], [0, 0], ENCODER_BUTTON_R, 0);
    assert_eq!(instance.values, before_switch);
    instance.event_buttons([0.5; 3], [0, 0], 0, ENCODER_BUTTON_R);
    instance.draw();
    TEXT.with(|text| assert!(text.borrow().iter().any(|(_, _, s)| s == "Push: Mode")));
    SHAPES.with(|shapes| {
        let shapes = shapes.borrow();
        assert!(!shapes.contains(&(SHAPE_RECTANGLE, 0, 0, 255, 7)));
        assert!(shapes.contains(&(SHAPE_RECTANGLE, 0, 0, 255, 12)));
        // Segment 3 remains selected after returning from Globals mode.
        assert!(shapes.contains(&(SHAPE_RECTANGLE, 63, 14, 95, 42)));
        assert_eq!(shapes.iter().filter(|s| s.0 == SHAPE_BOX).count(), 2);
    });
    HEADER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[15, 15, 15]));
    TRACE_LINES.with(|lines| {
        let lines = lines.borrow();
        assert!(lines.len() >= 255 && lines.len() <= 255 + 32);
        assert!(lines.iter().all(|line| line.4 == 15.0));
    });
    GRID_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[1; 71]));
    instance.change(segment_parameter(2, 0), -50);
    instance.change(segment_parameter(2, 2), -50);
    instance.change(segment_parameter(2, 1), -50);
    instance.event([0.508; 3], [0, 0]);
    for field in 0..3 {
        assert!(instance.values[segment_parameter(2, field)] > -50);
    }
    assert_eq!(&instance.values[..GLOBALS], &before_switch[..GLOBALS]);
    unsafe {
        assert_ne!(
            factory().has_custom_ui.unwrap()(instance.base) & ENCODER_BUTTON_R,
            0
        );
    }
}

#[test]
fn editor_selection_relative_pots_and_permanent_footer() {
    let mut instance = Instance::new(32);
    instance.event([0.5; 3], [0, 0]);
    instance.event([0.51; 3], [0, 1]);
    assert_eq!(editor_parameter(32, 0, 0), Some(WAVEFORM_START));
    assert!(instance.values[segment_parameter(0, 2)] > 0);
    assert_eq!(instance.values[SMOOTHING], 0);
    instance.event([0.51; 3], [31, 0]);
    let previous = instance.values[segment_parameter(31, 0)];
    instance.event([0.52; 3], [0, 0]);
    assert!(instance.values[segment_parameter(31, 0)] > previous);
    assert_eq!(editor_parameter(32, 31, 1), Some(WAVEFORM_START));
    let mut samples = [0.0; LAST_BUS * 64];
    for _ in 0..200 {
        instance.render(&mut samples);
    }
    instance.draw();
    TEXT.with(|text| {
        let text = text.borrow();
        for expected in ["Turn: Select", "Push: Mode"] {
            assert!(text.iter().any(|(_, y, s)| *y >= 57 && s == expected));
        }
        assert_eq!(text.len(), 5 + usize::from(cfg!(feature = "profiling")));
        assert!(text.iter().all(|(_, y, _)| *y != 57));
        for (x, prefix, field) in [(2, "Left", 0), (102, "Curve", 2), (190, "Right", 1)] {
            let value = instance.values[editor_parameter(32, 31, field).unwrap()];
            let expected = std::format!("{prefix} {value}");
            assert!(text
                .iter()
                .any(|(tx, y, s)| *tx == x && *y == 10 && s == &expected));
        }
    });
    SHAPES.with(|shapes| {
        let shapes = shapes.borrow();
        assert!(shapes.contains(&(SHAPE_LINE, 0, 50, 255, 50)));
        // Waveform lines cannot invade the footer, including the divider.
        for &(kind, _, y0, _, y1) in shapes.iter().filter(|s| s.0 == SHAPE_LINE && s.2 != 50) {
            assert_eq!(kind, SHAPE_LINE);
            assert!(y0 < 50 && y1 < 50);
        }
    });
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
#[no_mangle]
unsafe extern "C" fn NT_floatToString(buffer: *mut c_char, value: f32, _: i32) -> i32 {
    use std::string::ToString;
    let text = (value as i32).to_string();
    ptr::copy_nonoverlapping(text.as_ptr(), buffer.cast(), text.len());
    *buffer.add(text.len()) = 0;
    text.len() as i32
}
#[no_mangle]
unsafe extern "C" fn NT_intToString(buffer: *mut c_char, value: i32) -> i32 {
    use std::string::ToString;
    let text = value.to_string();
    ptr::copy_nonoverlapping(text.as_ptr(), buffer.cast(), text.len());
    *buffer.add(text.len()) = 0;
    text.len() as i32
}
#[no_mangle]
unsafe extern "C" fn NT_algorithmIndex(_: *const NtAlgorithm) -> i32 {
    0
}
#[no_mangle]
unsafe extern "C" fn NT_parameterOffset() -> u32 {
    0
}
#[no_mangle]
unsafe extern "C" fn NT_setParameterGrayedOut(_: u32, p: u32, gray: bool) {
    GRAY_CALLS.with(|calls| calls.borrow_mut().push((p as usize, gray)));
}
#[no_mangle]
unsafe extern "C" fn NT_setParameterFromUi(_: u32, p: u32, value: i16) {
    let base = CURRENT.with(|current| current.get());
    assert!(!base.is_null());
    let definition = *(*base).parameters.add(p as usize);
    *(*base).v.add(p as usize).cast_mut() = value.clamp(definition.min, definition.max);
    factory().parameter_changed.unwrap()(base, p as i32);
}
#[no_mangle]
unsafe extern "C" fn NT_setParameterFromAudio(index: u32, p: u32, value: i16) {
    NT_setParameterFromUi(index, p, value);
}
#[no_mangle]
unsafe extern "C" fn NT_drawText(x: i32, y: i32, text: *const c_char, color: i32, _: u8, size: u8) {
    if y == 10 {
        assert_eq!(size, TEXT_NORMAL);
        HEADER_COLORS.with(|colors| colors.borrow_mut().push(color));
    } else if y == 62 {
        assert_eq!(size, TEXT_NORMAL);
        FOOTER_COVER_AFTER_HELP.with(|covered| assert!(covered.get()));
    } else {
        assert_eq!(size, TEXT_TINY);
    }
    let value = String::from(CStr::from_ptr(text).to_str().unwrap());
    TEXT.with(|text| text.borrow_mut().push((x, y, value)));
}
#[no_mangle]
unsafe extern "C" fn NT_drawShapeI(kind: u8, x0: i32, y0: i32, x1: i32, y1: i32, color: i32) {
    if (kind, x0, y0, x1, y1) == (SHAPE_RECTANGLE, 0, 50, 255, 63) {
        let after_help = TEXT.with(|text| {
            text.borrow()
                .last()
                .is_some_and(|(_, _, text)| text == "Push: Mode")
        });
        FOOTER_COVER_AFTER_HELP.with(|covered| covered.set(after_help));
        assert_eq!(color, 0);
    }

    if y0 <= 42 && y1 <= 42 {
        if kind == SHAPE_LINE || kind == SHAPE_POINT {
            GRID_COLORS.with(|colors| colors.borrow_mut().push(color));
        }
        if kind == SHAPE_BOX {
            MARKER_COLORS.with(|colors| colors.borrow_mut().push(color));
        }
    }
    SHAPES.with(|shapes| shapes.borrow_mut().push((kind, x0, y0, x1, y1)));
}

#[no_mangle]
unsafe extern "C" fn NT_drawShapeF(kind: u8, x0: f32, y0: f32, x1: f32, y1: f32, color: f32) {
    assert_eq!(kind, SHAPE_LINE);
    assert!((0.0..=255.0).contains(&x0) && (0.0..=255.0).contains(&x1));
    assert!((14.0..=42.0).contains(&y0) && (14.0..=42.0).contains(&y1));
    TRACE_LINES.with(|lines| lines.borrow_mut().push((x0, y0, x1, y1, color)));
}

#[test]
fn shared_midi_output_normalizes_independent_notes_with_the_existing_gate_fades() {
    for voices in [1, 4, 8] {
        let mut mixed = Instance::with_voices(8, voices);
        mixed.change(MIDI_RELEASE, 25);
        mixed.change(OUTPUT_MODE, 1);
        mixed.change(MIDI_CHANNEL, 1);
        for voice in 0..voices {
            mixed.midi(0x90, 60 + voice as u8, 100);
        }
        let mut control = levels::LevelControl::new(48000);
        control.restart_for_mode_change();
        let mut references: Vec<_> = (0..voices)
            .map(|voice| {
                let mut mono = Instance::new(8);
                mono.change(MIDI_RELEASE, 25);
                mono.change(OUTPUT_MODE, 1);
                mono.change(MIDI_CHANNEL, 1);
                mono.midi(0x90, 60 + voice as u8, 100);
                mono
            })
            .collect();
        let mut reference_gate = 0.0f32;
        for block in 0..160 {
            if block == 120 {
                for voice in 0..voices {
                    mixed.midi(0x80, 60 + voice as u8, 0);
                }
                for (voice, mono) in references.iter_mut().enumerate() {
                    mono.midi(0x80, 60 + voice as u8, 0);
                }
            }
            let mut samples = [0.0; LAST_BUS * 24];
            mixed.render(&mut samples);
            let mut expected = [0.0; 24];
            let mut powers = [0.0; 24];
            for mono in &mut references {
                let mut buffer = [0.0; LAST_BUS * 24];
                mono.render(&mut buffer);
                for ((sum, power), &v) in expected
                    .iter_mut()
                    .zip(powers.iter_mut())
                    .zip(&buffer[12 * 24..13 * 24])
                {
                    *sum += v;
                    *power += v * v;
                }
            }
            let mut weights = [0.0; 24];
            for weight in &mut weights {
                reference_gate = if block < 120 {
                    (reference_gate + 1.0 / 240.0).min(1.0)
                } else {
                    (reference_gate - 1.0 / 1200.0).max(0.0)
                };
                *weight = voices as f32 * reference_gate * reference_gate;
            }
            for ((chunk, powers), weights) in expected
                .chunks_mut(levels::CHUNK)
                .zip(powers.chunks(levels::CHUNK))
                .zip(weights.chunks(levels::CHUNK))
            {
                control.process(
                    chunk,
                    powers.iter().sum(),
                    weights.iter().sum::<f32>() / weights.len() as f32,
                    levels::PEAK_CEILING,
                );
            }
            for (&actual, &expected) in samples[12 * 24..13 * 24].iter().zip(&expected) {
                assert!((actual - expected).abs() < 0.00001);
            }
        }
    }
}

// Snapshot fields, not struct bytes: ABI padding is not initialized data.
fn persistence_metadata(instance: &Instance) -> Vec<String> {
    let mut result = Vec::new();
    unsafe {
        let factory = factory();
        assert!(factory.serialise.is_none());
        assert!(factory.deserialise.is_none());
        assert!(factory.parameter_string.is_some());
        assert!(factory.parameter_ui_prefix.is_some());
        assert!(factory.string_parameter_buffer.is_none());
        let mut names = Vec::new();
        for index in 0..instance.values.len() {
            let p = &*(*instance.base).parameters.add(index);
            let name = CStr::from_ptr(p.name).to_str().unwrap();
            assert!(!name.is_empty());
            let qualified = std::format!("{}{name}", parameter_prefix(instance, index as i32));
            assert!(
                !names.contains(&qualified),
                "duplicate displayed parameter name {qualified}"
            );
            names.push(qualified);
            assert!(p.min <= p.default && p.default <= p.max);
            assert!((p.min..=p.max).contains(&instance.values[index]));
            result.push(std::format!(
                "{index}:{name}:{}:{}:{}:{}:{}:{:p}:{:p}",
                p.min,
                p.max,
                p.default,
                p.unit,
                p.scaling,
                p.name,
                p.enum_strings
            ));
            if p.unit == UNIT_ENUM {
                assert!(!p.enum_strings.is_null());
                for value in p.min..=p.max {
                    let label = CStr::from_ptr(*p.enum_strings.add(value as usize))
                        .to_str()
                        .unwrap();
                    assert!(!label.is_empty());
                    result.push(std::format!("enum:{value}:{label}"));
                }
            } else {
                assert!(p.enum_strings.is_null());
            }
        }
        let pages = &*(*instance.base).parameter_pages;
        assert_eq!(
            pages.num_pages as usize,
            (instance.values.len() - VOICE_START) / VOICE_PARAMETERS + 5
        );
        for i in 0..pages.num_pages as usize {
            let page = &*pages.pages.add(i);
            let name = CStr::from_ptr(page.name).to_str().unwrap();
            let rows = core::slice::from_raw_parts(page.params, page.num_params as usize);
            assert!(rows.iter().all(|&p| (p as usize) < instance.values.len()));
            result.push(std::format!(
                "page:{name}:{}:{:p}:{rows:?}",
                page.group,
                page.params
            ));
        }
    }
    instance.assert_guard();
    result
}

#[test]
fn preset_parameter_metadata_is_complete_and_stable_during_voice_and_shape_activity() {
    for (segments, voices) in [(8, 1), (16, 10), (32, 10)] {
        let mut instance = Instance::with_voices(segments, voices);
        let initial = persistence_metadata(&instance);
        for index in 0..instance.values.len() {
            let (min, max) = unsafe {
                let p = &*(*instance.base).parameters.add(index);
                (p.min, p.max)
            };
            for value in [min, max] {
                instance.change(index, value);
                assert_eq!(persistence_metadata(&instance), initial);
            }
        }
        instance.change(LEFT_OUTPUT, 13);
        instance.change(OUTPUT_MODE, 1);
        instance.change(MIDI_CHANNEL, 1);
        instance.change(SMOOTHING, 37);
        for voice in 0..voices {
            instance.midi(0x90, 48 + voice as u8, 100);
        }
        let mut samples = [0.0; LAST_BUS * 24];
        for block in 0..400 {
            if block % 16 == 0 {
                instance.change(
                    curve_parameter(block / 16 % segments as usize),
                    (block as i16 % 201) - 100,
                );
                instance.change(WARP_AMOUNT, (block as i16 % 201) - 100);
            }
            instance.render(&mut samples);
            instance.draw();
            assert_eq!(persistence_metadata(&instance), initial);
        }
    }
}

#[test]
fn midi_grays_only_cv_controls_and_restores_them_without_resetting_values() {
    for voices in [1, 10] {
        let mut instance = Instance::with_voices(8, voices);
        instance.change(PITCH, 69);
        instance.change(voice_transpose_parameter(0), 7);
        instance.change(voice_fine_parameter(0), -25);
        let original = instance.values.clone();
        let mut samples = [0.0; LAST_BUS * 24];
        instance.render(&mut samples);
        let mut expected = std::vec![(PITCH, false), (MIDI_ATTACK, true), (MIDI_RELEASE, true)];
        for voice in 0..voices as usize {
            expected.push((voice_pitch_parameter(voice), false));
            expected.push((voice_midi_group_parameter(voice), true));
        }
        let mut initial_expected = expected.clone();
        for voice in 0..voices as usize {
            initial_expected.push((voice_pan_parameter(voice), true));
        }
        GRAY_CALLS.with(|calls| assert_eq!(&*calls.borrow(), &initial_expected));
        GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
        instance.render(&mut samples);
        GRAY_CALLS.with(|calls| assert!(calls.borrow().is_empty()));
        instance.change(MIDI_CHANNEL, 1);
        for item in &mut expected {
            item.1 = !item.1;
        }
        GRAY_CALLS.with(|calls| assert_eq!(&*calls.borrow(), &expected));
        GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
        instance.change(MIDI_CHANNEL, 4);
        instance.render(&mut samples);
        GRAY_CALLS.with(|calls| assert!(calls.borrow().is_empty()));
        instance.change(MIDI_CHANNEL, 0);
        for item in &mut expected {
            item.1 = !item.1;
        }
        GRAY_CALLS.with(|calls| assert_eq!(&*calls.borrow(), &expected));
        assert_eq!(instance.values, original);
    }
}

#[test]
fn restored_midi_mode_grays_controls_on_first_audio_callback() {
    let mut instance = Instance::with_voices(32, 10);
    // Model a host restoring values before invoking any parameter notifications.
    instance.values[MIDI_CHANNEL] = 1;
    instance.render(&mut [0.0; LAST_BUS * 24]);
    GRAY_CALLS.with(|calls| {
        let calls = calls.borrow();
        assert_eq!(calls.len(), 3 + 10 * 3);
        assert!(calls.iter().all(|&(p, gray)| gray
            == !(p == MIDI_ATTACK
                || p == MIDI_RELEASE
                || (0..MAX_VOICES).any(|v| p == voice_midi_group_parameter(v)))));
        assert!(!calls.iter().any(|&(p, _)| p == FINE || p == LEVEL));
    });
}

#[test]
fn midi_pitch_pot_is_inactive_and_dim_but_fine_and_level_remain_editable() {
    let mut instance = Instance::new(8);
    instance.change(PITCH, 69);
    instance.change(MIDI_CHANNEL, 1);
    instance.event([0.5; 3], [0, 0]);
    instance.click_right([0.5; 3]);
    instance.click_right([0.5; 3]);
    for tick in 1..=10 {
        instance.event([0.5 + tick as f32 * 0.005; 3], [0, 0]);
    }
    assert_eq!(instance.values[PITCH], 69);
    assert!(instance.values[FINE] > 0);
    assert!(instance.values[LEVEL] > levels::LEVEL_DEFAULT);
    instance.draw();
    HEADER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[5, 15, 15]));
    instance.change(MIDI_CHANNEL, 0);
    for pot in [0.555, 0.560, 0.565] {
        instance.event([pot; 3], [0, 0]);
    }
    assert!(instance.values[PITCH] > 69);
    instance.draw();
    HEADER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[15, 15, 15]));
}

// Verify the actual render path's selected frequency, allowing normal f32
// rounding when cents and semitones are combined in different orders.
fn assert_cached_octave(instance: &Instance, voice: usize, expected: f32) {
    let context = unsafe { Callback::<Audio>::from_raw(instance.base) };
    let actual = context.state().oscillators[voice].target_frequency_for_test();
    let rate = context.state().sample_rate;
    let expected_hz = (440.0 * 2.0f32.powf(expected)).max(0.01).min(rate * 0.45);
    // Observe the real target phase step, including the existing integer LSB.
    let phase_lsb_hz = rate / 4_294_967_296.0;
    assert!(
        (actual - expected_hz).abs() < expected_hz * 0.000001 + phase_lsb_hz,
        "expected {expected_hz} Hz, got {actual}"
    );
}

#[test]
fn per_voice_tuning_applies_transpose_in_both_modes_but_midi_ignores_root_and_cv() {
    let mut instance = Instance::with_voices(8, 2);
    instance.change(PITCH, 69);
    instance.change(FINE, 25);
    instance.change(voice_fine_parameter(0), -25);
    instance.change(voice_fine_parameter(1), 25);
    instance.change(voice_transpose_parameter(0), 12);
    instance.change(voice_transpose_parameter(1), -12);
    instance.change(voice_pitch_parameter(0), 1);
    instance.change(voice_pitch_parameter(1), 2);
    let mut samples = [0.0; LAST_BUS * 24];
    samples[..24].fill(1.0);
    samples[24..48].fill(2.0);
    instance.render(&mut samples);
    assert_cached_octave(&instance, 0, 2.0);
    assert_cached_octave(&instance, 1, 1.0 + 50.0 / 1200.0);
    instance.change(MIDI_CHANNEL, 1);
    instance.change(PITCH, 0);
    instance.midi(0x90, 69, 100);
    instance.midi(0x90, 81, 100);
    instance.render(&mut samples);
    assert_cached_octave(&instance, 0, 1.0);
    assert_cached_octave(&instance, 1, 50.0 / 1200.0);
    instance.change(voice_transpose_parameter(1), 48);
    samples[..48].fill(-5.0);
    instance.render(&mut samples);
    assert_cached_octave(&instance, 1, 5.0 + 50.0 / 1200.0);
    instance.change(voice_fine_parameter(1), -25);
    instance.render(&mut samples);
    assert_cached_octave(&instance, 1, 5.0);
}

#[test]
fn voice_prefixes_cover_only_voice_parameters_and_respect_small_buffers() {
    let instance = Instance::with_voices(8, 10);
    for parameter in [
        LEFT_OUTPUT as i32,
        OUTPUT_MODE as i32,
        PITCH as i32,
        FINE as i32,
        MIDI_CHANNEL as i32,
        WAVEFORM_START as i32,
        -1,
        i32::MAX,
    ] {
        assert_eq!(parameter_prefix(&instance, parameter), "");
    }
    let mut context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
    for voice in [0, 9] {
        let parameter = voice_transpose_parameter(voice) as i32;
        let expected = std::format!("V{}:", voice + 1);
        for capacity in 0..=6 {
            let mut buffer = [0x55 as c_char; 8];
            let length = Wavesmith::prefix(&mut context, parameter, &mut buffer[..capacity]);
            assert!(buffer[capacity..].iter().all(|&c| c == 0x55 as c_char));
            if capacity <= expected.len() {
                assert_eq!(length, 0);
            } else {
                assert_eq!(length as usize, expected.len());
                assert_eq!(
                    unsafe { CStr::from_ptr(buffer.as_ptr()).to_str().unwrap() },
                    expected
                );
            }
        }
    }
}

#[test]
fn pot_push_resets_current_control_once_per_press_and_preserves_pickup() {
    for (segments, voices) in [(2, 1), (8, 1), (32, 10)] {
        for selected in [0, segments - 1] {
            for mode in 0..3 {
                for pot in 0..3 {
                    let mut instance = Instance::with_voices(segments, voices);
                    instance.event([0.8; 3], [selected as i8, 0]);
                    for _ in 0..mode {
                        instance.click_right([0.8; 3]);
                    }
                    let n = segments as usize;
                    let parameter = match mode {
                        0 => editor_parameter(n, selected as usize, [0, 2, 1][pot]).unwrap(),
                        1 => [WARP_POSITION, WARP_AMOUNT, SMOOTHING][pot],
                        _ => [PITCH, FINE, LEVEL][pot],
                    };
                    let definition = unsafe { *(*instance.base).parameters.add(parameter) };
                    let reset_value = if mode == 0 && pot != 1 {
                        0
                    } else {
                        definition.default
                    };
                    let changed = if reset_value == definition.min {
                        definition.max
                    } else {
                        definition.min
                    };
                    instance.change(parameter, changed);
                    let mut expected = instance.values.clone();
                    expected[parameter] = reset_value;
                    let button = [POT_BUTTON_L, POT_BUTTON_C, POT_BUTTON_R][pot];
                    let mut pots = [0.8; 3];
                    pots[pot] = 0.805;
                    instance.event_buttons(pots, [0, 0], button, 0);
                    assert_eq!(instance.values, expected);
                    // A held button must not repeatedly overwrite external edits.
                    instance.change(parameter, changed);
                    instance.event_buttons(pots, [0, 0], button, button);
                    assert_eq!(instance.values[parameter], changed);
                    instance.event_buttons(pots, [0, 0], 0, button);
                    assert_eq!(instance.values[parameter], changed);
                    instance.event_buttons(pots, [0, 0], button, 0);
                    assert_eq!(instance.values, expected);
                    // Releasing at a different position must not cause a jump.
                    pots[pot] = 0.9;
                    instance.event_buttons(pots, [0, 0], 0, button);
                    instance.event(pots, [0, 0]);
                    assert_eq!(instance.values, expected);
                    unsafe {
                        let mask = factory().has_custom_ui.unwrap()(instance.base);
                        assert_eq!(
                            mask & (POT_BUTTON_L | POT_BUTTON_C | POT_BUTTON_R),
                            POT_BUTTON_L | POT_BUTTON_C | POT_BUTTON_R
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn pot_resets_keep_midi_pitch_locked_and_fine_and_level_active() {
    let mut instance = Instance::new(8);
    instance.change(PITCH, 100);
    instance.change(FINE, 50);
    instance.change(LEVEL, -2);
    instance.change(MIDI_CHANNEL, 1);
    instance.event([0.8; 3], [0, 0]);
    instance.click_right([0.8; 3]);
    instance.click_right([0.8; 3]);
    let buttons = POT_BUTTON_L | POT_BUTTON_C | POT_BUTTON_R;
    instance.event_buttons([0.8; 3], [0, 0], buttons, 0);
    assert_eq!(instance.values[PITCH], 100);
    assert_eq!(instance.values[FINE], 0);
    assert_eq!(instance.values[LEVEL], levels::LEVEL_DEFAULT);
    instance.event_buttons([0.8; 3], [0, 0], 0, buttons);
    instance.change(MIDI_CHANNEL, 0);
    instance.event_buttons([0.8; 3], [0, 0], POT_BUTTON_L, 0);
    assert_eq!(instance.values[PITCH], 48);
}

#[test]
fn midi_bend_reaches_audio_per_channel_combines_with_fine_and_reuses_pitch_cache() {
    let mut instance = Instance::with_voices(8, 3);
    instance.change(OUTPUT_MODE, 1);
    instance.change(MIDI_CHANNEL, 1);
    instance.change(FINE, 25);
    instance.change(voice_fine_parameter(0), -25);
    instance.change(voice_fine_parameter(1), 25);
    instance.midi(0xe0, 0, 96); // Channel 1, +1 semitone, before Note On.
    instance.midi(0x90, 69, 100);
    instance.midi(0x90, 81, 100);
    instance.midi(0x91, 69, 100);
    let mut samples = [0.0; LAST_BUS * 24];
    instance.render(&mut samples);
    assert_cached_octave(&instance, 0, 1.0 / 12.0);
    assert_cached_octave(&instance, 1, 1.0 + 1.0 / 12.0 + 50.0 / 1200.0);
    assert_cached_octave(&instance, 2, 25.0 / 1200.0);
    instance.midi(0xe1, 0, 0); // Channel 2, -2 semitones on held note.
    instance.midi(0xe0, 1, 64); // Low byte matters: just above neutral.
    instance.render(&mut samples);
    assert_cached_octave(&instance, 0, 1.0 / (8192.0 * 6.0));
    assert_cached_octave(&instance, 1, 1.0 + 1.0 / (8192.0 * 6.0) + 50.0 / 1200.0);
    assert_cached_octave(&instance, 2, -2.0 / 12.0 + 25.0 / 1200.0);
    let before = dsp::playback_work_counts().1;
    for _ in 0..10 {
        instance.render(&mut samples);
    }
    assert_eq!(dsp::playback_work_counts().1, before);
    instance.midi(0xe0, 0, 64);
    instance.render(&mut samples);
    assert_cached_octave(&instance, 0, 0.0);
    instance.change(MIDI_CHANNEL, 0);
    instance.change(PITCH, 69);
    instance.midi(0xe0, 0, 0);
    instance.render(&mut samples);
    assert_cached_octave(&instance, 0, 0.0); // CV mode unaffected.
    assert_cached_octave(&instance, 1, 50.0 / 1200.0);
}

#[test]
fn normalized_add_mode_preserves_existing_bus_audio_exactly() {
    let mut add = Instance::with_voices(8, 10);
    let mut replace = Instance::with_voices(8, 10);
    replace.change(OUTPUT_MODE, 1);
    for _ in 0..500 {
        let mut a = [0.0; LAST_BUS * 24];
        let mut b = [0.0; LAST_BUS * 24];
        a[12 * 24..13 * 24].fill(3.0);
        add.render(&mut a);
        replace.render(&mut b);
        for (&a, &b) in a[12 * 24..13 * 24].iter().zip(&b[12 * 24..13 * 24]) {
            assert!((a - (3.0 + b)).abs() < 0.000001);
            assert!(b.abs() <= levels::PEAK_CEILING + 0.000001);
        }
    }
}

#[test]
fn midi_to_cv_crossfades_without_a_step_or_full_pool_gain_burst() {
    for mode in [0, 1] {
        let mut instance = Instance::with_voices(8, 10);
        instance.change(OUTPUT_MODE, mode);
        for boundary in 0..8 {
            instance.change(boundary_parameter(boundary), 100);
        }
        instance.change(MIDI_CHANNEL, 1);
        instance.midi(0x90, 69, 100);
        let mut samples = [0.0; LAST_BUS * 24];
        let baseline = if mode == 0 { 3.0 } else { 0.0 };
        for _ in 0..160 {
            samples.fill(baseline);
            instance.render(&mut samples);
        }
        let previous = samples[13 * 24 - 1];
        assert!((previous - baseline - 5.0 * levels::DEFAULT_GAIN).abs() < 0.0001);
        instance.change(MIDI_CHANNEL, 0);
        for block in 0..160 {
            samples.fill(baseline);
            instance.render(&mut samples);
            let output = &samples[12 * 24..13 * 24];
            if block == 0 {
                assert_eq!(output[0], previous);
            }
            assert!(output
                .iter()
                .all(|v| (*v - baseline).abs() <= 5.0 * levels::DEFAULT_GAIN + 0.0001));
        }
        assert!((samples[13 * 24 - 1] - baseline - 5.0 * levels::DEFAULT_GAIN).abs() < 0.0001);
        let previous = samples[13 * 24 - 1];
        instance.change(MIDI_CHANNEL, 1);
        samples.fill(baseline);
        instance.render(&mut samples);
        assert_eq!(samples[12 * 24], previous);
        for _ in 0..60 {
            samples.fill(baseline);
            instance.render(&mut samples);
        }
        assert!(samples[12 * 24..13 * 24].iter().all(|&v| v == baseline));
    }
}

#[test]
fn decibel_level_metadata_display_reset_and_rendered_amplitude_match() {
    let mut instance = Instance::new(8);
    let definition = unsafe { &*(*instance.base).parameters.add(LEVEL) };
    assert_eq!(
        (
            definition.min,
            definition.max,
            definition.default,
            definition.unit
        ),
        (levels::LEVEL_MIN, 0, levels::LEVEL_DEFAULT, UNIT_DB_MIN_INF)
    );
    instance.change(OUTPUT_MODE, 1);
    for boundary in 0..8 {
        instance.change(boundary_parameter(boundary), 100);
    }
    instance.click_right([0.5; 3]);
    instance.click_right([0.5; 3]);
    let mut samples = [0.0; LAST_BUS * 24];
    for db in [0, -6, -12, -59, levels::LEVEL_MIN] {
        instance.change(LEVEL, db);
        for _ in 0..1800 {
            instance.render(&mut samples);
        }
        let expected = if db == levels::LEVEL_MIN {
            0.0
        } else {
            5.0 * 10.0f32.powf(db as f32 / 20.0)
        };
        assert!((samples[13 * 24 - 1] - expected).abs() < 0.0001);
        instance.draw();
        let label = if db == levels::LEVEL_MIN {
            String::from("Level -inf")
        } else {
            std::format!("Level {db} dB")
        };
        TEXT.with(|text| {
            assert!(text
                .borrow()
                .iter()
                .any(|&(x, y, ref value)| x == 180 && y == 10 && value == &label))
        });
    }
    instance.event_buttons([0.5; 3], [0, 0], POT_BUTTON_R, 0);
    assert_eq!(instance.values[LEVEL], -6);
}

#[test]
fn none_assignment_masks_midi_allocation_before_audio_and_does_not_resume_old_keys() {
    let mut instance = Instance::with_voices(8, 10);
    instance.change(OUTPUT_MODE, 1);
    instance.change(MIDI_CHANNEL, 1);
    for voice in 0..10 {
        instance.change(
            voice_midi_group_parameter(voice),
            if voice % 2 == 1 { voice as i16 + 1 } else { 0 },
        );
    }
    for note in 60..65 {
        instance.midi(0x90, note, 100);
    }
    {
        let context = unsafe { Callback::<Audio>::from_raw(instance.base) };
        for (voice, note) in context.state().midi.groups.iter().enumerate() {
            assert_eq!(note.sounding(), voice % 2 == 1);
        }
    }
    instance.midi(0x90, 72, 100); // Sixth note steals group 2, never an inactive voice.
    instance.change(voice_midi_group_parameter(1), 0);
    instance.midi(0x90, 74, 100); // Disable must apply even before the next step.
    {
        let context = unsafe { Callback::<Audio>::from_raw(instance.base) };
        assert_eq!(context.state().midi.groups[3].pitch(), 74);
    }
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..160 {
        instance.render(&mut samples);
    }
    let context = unsafe { Callback::<Audio>::from_raw(instance.base) };
    assert!(!context.state().midi.groups[1].sounding());
    instance.change(voice_midi_group_parameter(1), 2);
    instance.render(&mut samples);
    let context = unsafe { Callback::<Audio>::from_raw(instance.base) };
    assert!(!context.state().midi.groups[1].sounding());
    instance.midi(0x90, 76, 100);
    instance.render(&mut samples);
    let context = unsafe { Callback::<Audio>::from_raw(instance.base) };
    assert_eq!(context.state().midi.groups[1].pitch(), 76);
    for voice in 0..10 {
        instance.change(voice_midi_group_parameter(voice), 0);
    }
    for _ in 0..30 {
        instance.render(&mut samples);
    }
    assert!(samples[12 * 24..13 * 24].iter().all(|&v| v == 0.0));
}

#[test]
fn voice_respecification_preserves_all_shared_waveform_and_surviving_voice_values() {
    assert_eq!(Wavesmith::SPECIFICATIONS[0].name, c"Voices");
    for old_voices in 1..=MAX_VOICES as i32 {
        let mut old = Instance::with_specified_voices(old_voices);
        for (index, value) in old.values.iter_mut().enumerate() {
            let d = unsafe { &*(*old.base).parameters.add(index) };
            *value = d.min + (index % (d.max as i32 - d.min as i32 + 1) as usize) as i16;
        }
        for new_voices in 1..=MAX_VOICES as i32 {
            let mut new = Instance::with_specified_voices(new_voices);
            let retained = old.values.len().min(new.values.len());
            new.values[..retained].copy_from_slice(&old.values[..retained]);
            for parameter in 0..retained {
                let old_name =
                    unsafe { CStr::from_ptr((*(*old.base).parameters.add(parameter)).name) };
                let new_name =
                    unsafe { CStr::from_ptr((*(*new.base).parameters.add(parameter)).name) };
                assert_eq!(
                    old_name, new_name,
                    "{old_voices}->{new_voices}, ordinal {parameter}"
                );
                assert_eq!(
                    parameter_prefix(&old, parameter as i32),
                    parameter_prefix(&new, parameter as i32)
                );
                assert_eq!(old.values[parameter], new.values[parameter]);
            }
            assert_eq!(
                new.values.len(),
                VOICE_START + new_voices as usize * VOICE_PARAMETERS
            );
            for parameter in WAVEFORM_START..VOICE_START {
                assert_eq!(old.values[parameter], new.values[parameter]);
            }
        }
    }
}

#[test]
fn stereo_center_and_endpoints_match_mono_and_generate_each_voice_only_once() {
    for position in [0, 50, 100] {
        let mut mono = Instance::with_voices(8, 5);
        let mut stereo = Instance::with_voices(8, 5);
        mono.change(OUTPUT_MODE, 1);
        stereo.change(OUTPUT_MODE, 1);
        stereo.change(RIGHT_OUTPUT, 14);
        for voice in 0..5 {
            stereo.change(voice_pan_parameter(voice), position);
        }
        for block in 0..480 {
            let mut a = [0.0; LAST_BUS * 24];
            let mut b = [0.0; LAST_BUS * 24];
            mono.render(&mut a);
            stereo.render(&mut b);
            for ((&m, &l), &r) in a[12 * 24..13 * 24]
                .iter()
                .zip(&b[12 * 24..13 * 24])
                .zip(&b[13 * 24..14 * 24])
            {
                let (left, right) = match position {
                    0 => (1.0, 0.0),
                    100 => (0.0, 1.0),
                    _ => (
                        core::f32::consts::FRAC_1_SQRT_2,
                        core::f32::consts::FRAC_1_SQRT_2,
                    ),
                };
                if block < 400 {
                    continue;
                } // Different per-channel headroom can limit the initial unnormalized chord.
                assert!(
                    (l - m * left).abs() < 0.00002,
                    "position {position}, block {block}"
                );
                assert!((r - m * right).abs() < 0.00002);
            }
        }
        let before = dsp::playback_work_counts();
        stereo.render(&mut [0.0; LAST_BUS * 24]);
        assert_eq!(dsp::playback_work_counts().0 - before.0, 5 * 24);
        assert_eq!(dsp::playback_work_counts().1, before.1);
    }
}

#[test]
fn right_only_and_duplicate_bus_routing_are_full_level_mono_and_ignore_stored_pan() {
    for (left, right) in [(0, 14), (14, 14)] {
        for mode in [0, 1] {
            let mut reference = Instance::with_voices(8, 3);
            let mut actual = Instance::with_voices(8, 3);
            reference.change(LEFT_OUTPUT, 14);
            reference.change(OUTPUT_MODE, mode);
            actual.change(LEFT_OUTPUT, left);
            actual.change(RIGHT_OUTPUT, right);
            actual.change(OUTPUT_MODE, mode);
            for voice in 0..3 {
                actual.change(voice_pan_parameter(voice), if voice == 0 { 0 } else { 100 });
            }
            for _ in 0..180 {
                let mut a = [2.0; LAST_BUS * 24];
                let mut b = a;
                reference.render(&mut a);
                actual.render(&mut b);
                assert_eq!(a, b);
            }
        }
    }
}

#[test]
fn pan_graying_follows_distinct_output_buses_and_preserves_values() {
    let mut instance = Instance::new(8);
    instance.change(voice_pan_parameter(0), 17);
    instance.render(&mut [0.0; LAST_BUS * 24]);
    GRAY_CALLS.with(|calls| assert!(calls.borrow().contains(&(voice_pan_parameter(0), true))));
    for (left, right, gray) in [
        (13, 14, false),
        (0, 14, true),
        (14, 14, true),
        (14, 13, false),
        (14, 0, true),
        (0, 0, true),
    ] {
        GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
        instance.change(LEFT_OUTPUT, left);
        instance.change(RIGHT_OUTPUT, right);
        instance.render(&mut [0.0; LAST_BUS * 24]);
        let context = unsafe { Callback::<Audio>::from_raw(instance.base) };
        assert_eq!(context.state().grayed_for_stereo, Some(!gray));
        assert_eq!(instance.values[voice_pan_parameter(0)], 17);
        GRAY_CALLS.with(|calls| {
            for &(p, _) in calls.borrow().iter() {
                assert!((0..MAX_VOICES).any(|v| p == voice_pan_parameter(v)));
            }
        });
    }
}

#[test]
fn stereo_snapshots_both_cv_inputs_before_writing_and_preserves_additive_audio() {
    let mut add = Instance::with_voices(8, 2);
    let mut replace = Instance::with_voices(8, 2);
    for instance in [&mut add, &mut replace] {
        instance.change(PITCH, 69);
        instance.change(LEFT_OUTPUT, 1);
        instance.change(RIGHT_OUTPUT, 2);
        instance.change(voice_pitch_parameter(0), 1);
        instance.change(voice_pitch_parameter(1), 2);
        instance.change(voice_pan_parameter(0), 0);
        instance.change(voice_pan_parameter(1), 100);
    }
    replace.change(OUTPUT_MODE, 1);
    for _ in 0..240 {
        let mut a = [0.0; LAST_BUS * 24];
        let mut b = a;
        a[..24].fill(0.5);
        a[24..48].fill(-0.25);
        b.copy_from_slice(&a);
        add.render(&mut a);
        replace.render(&mut b);
        assert_cached_octave(&add, 0, 0.5);
        assert_cached_octave(&add, 1, -0.25);
        for (&a, &b) in a[..24].iter().zip(&b[..24]) {
            assert!((a - b - 0.5).abs() < 0.000001);
        }
        for (&a, &b) in a[24..48].iter().zip(&b[24..48]) {
            assert!((a - b + 0.25).abs() < 0.000001);
        }
        assert!(a[48..].iter().all(|&v| v == 0.0));
    }
}

#[test]
fn right_only_midi_stays_playing_when_left_is_disabled_and_both_none_stop_work() {
    let mut instance = Instance::with_voices(8, 2);
    instance.change(OUTPUT_MODE, 1);
    instance.change(MIDI_CHANNEL, 1);
    instance.midi(0x90, 69, 100);
    instance.midi(0x90, 76, 100);
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..160 {
        instance.render(&mut samples);
    }
    instance.change(RIGHT_OUTPUT, 14);
    instance.change(LEFT_OUTPUT, 0);
    for _ in 0..160 {
        instance.render(&mut samples);
    }
    let context = unsafe { Callback::<Audio>::from_raw(instance.base) };
    assert!(context.state().midi.groups[0].sounding());
    assert!(context.state().midi.groups[1].sounding());
    assert!(samples[13 * 24..14 * 24].iter().any(|v| v.abs() > 0.01));
    instance.change(RIGHT_OUTPUT, 0);
    let before = dsp::playback_work_counts();
    samples.fill(2.0);
    instance.render(&mut samples);
    assert_eq!(dsp::playback_work_counts(), before);
    assert!(samples.iter().all(|&v| v == 2.0));
}

#[test]
fn mono_right_to_stereo_preserves_output_continuity_and_centers_the_voice() {
    let mut instance = Instance::new(8);
    instance.change(PITCH, 57); // Keep the continuity fixture independent of pitch defaults.
    instance.change(LEFT_OUTPUT, 0);
    instance.change(RIGHT_OUTPUT, 14);
    instance.change(OUTPUT_MODE, 1);
    for boundary in 0..8 {
        instance.change(boundary_parameter(boundary), 100);
    }
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..160 {
        samples.fill(0.0);
        instance.render(&mut samples);
    }
    let previous = samples[14 * 24 - 1];
    assert!((previous - 5.0 * levels::DEFAULT_GAIN).abs() < 0.0001);
    instance.change(LEFT_OUTPUT, 13);
    samples.fill(0.0);
    instance.render(&mut samples);
    assert_eq!(samples[12 * 24], 0.0);
    assert_eq!(samples[13 * 24], previous);
    for _ in 0..80 {
        samples.fill(0.0);
        instance.render(&mut samples);
    }
    let expected = 5.0 * levels::DEFAULT_GAIN * core::f32::consts::FRAC_1_SQRT_2;
    assert!((samples[13 * 24 - 1] - expected).abs() < 0.0001);
    assert!((samples[14 * 24 - 1] - expected).abs() < 0.0001);
    let previous = samples[13 * 24 - 1];
    instance.change(RIGHT_OUTPUT, 0);
    samples.fill(0.0);
    instance.render(&mut samples);
    assert_eq!(samples[12 * 24], previous);
    for _ in 0..80 {
        samples.fill(0.0);
        instance.render(&mut samples);
    }
    assert!((samples[13 * 24 - 1] - 5.0 * levels::DEFAULT_GAIN).abs() < 0.0001);
    assert!(samples[13 * 24..14 * 24].iter().all(|&v| v == 0.0));
}

#[test]
fn moving_only_right_stereo_bus_leaves_left_sample_exact_and_keeps_level_history() {
    for voices in [1, 5] {
        let mut reference = Instance::with_voices(8, voices);
        let mut moved = Instance::with_voices(8, voices);
        for instance in [&mut reference, &mut moved] {
            instance.change(OUTPUT_MODE, 1);
            instance.change(RIGHT_OUTPUT, 14);
            for voice in 0..voices as usize {
                instance.change(voice_transpose_parameter(voice), voice as i16 * 3);
            }
        }
        for block in 0..1600 {
            if block >= 1000 && block % 24 == 0 {
                moved.change(RIGHT_OUTPUT, 14 + ((block / 24) % 10) as i16);
            }
            let mut a = [0.0; LAST_BUS * 24];
            let mut b = a;
            reference.render(&mut a);
            moved.render(&mut b);
            assert_eq!(
                &a[12 * 24..13 * 24],
                &b[12 * 24..13 * 24],
                "voices {voices}, block {block}"
            );
        }
    }
}

#[test]
fn entering_and_leaving_centered_stereo_keeps_the_live_waveform_without_transient_boost() {
    let mut reference = Instance::new(8);
    let mut switched = Instance::new(8);
    reference.change(OUTPUT_MODE, 1);
    switched.change(OUTPUT_MODE, 1);
    let mut a = [0.0; LAST_BUS * 24];
    let mut b = a;
    for block in 0..1200 {
        if block == 400 {
            switched.change(RIGHT_OUTPUT, 14);
        }
        if block == 800 {
            switched.change(RIGHT_OUTPUT, 0);
        }
        a.fill(0.0);
        b.fill(0.0);
        reference.render(&mut a);
        switched.render(&mut b);
        for (&mono, &left) in a[12 * 24..13 * 24].iter().zip(&b[12 * 24..13 * 24]) {
            // Live gain interpolation must preserve phase and remain between
            // mono unity gain and the centered stereo gain, including crossings.
            assert!(
                left.abs() <= mono.abs() + 0.00002,
                "block {block}: {left} vs {mono}"
            );
            assert!(left.abs() + 0.00002 >= mono.abs() * core::f32::consts::FRAC_1_SQRT_2);
            assert!(
                left * mono >= -0.000001,
                "routing transition must not freeze waveform phase"
            );
        }
    }
}

#[no_mangle]
unsafe extern "C" fn NT_getCpuCycleCount() -> u32 {
    CYCLE_COUNTER.with(|counter| {
        let result = counter.get();
        counter.set(result.wrapping_add(100));
        result
    })
}

#[cfg(feature = "profiling")]
#[test]
fn render_cycle_meter_averages_complete_windows_without_overflow() {
    let mut meter = RenderCycles::new();
    for _ in 0..255 {
        meter.record(u32::MAX);
    }
    assert_eq!(meter.average, 0);
    meter.record(u32::MAX);
    assert_eq!(meter.average, u32::MAX);
    for _ in 0..256 {
        meter.record(12345);
    }
    assert_eq!(meter.average, 12345);
    assert_eq!((meter.total, meter.callbacks), (0, 0));
}

#[cfg(feature = "profiling")]
#[test]
fn cycle_overlay_covers_preserved_help_and_sums_chunks_across_counter_wrap() {
    for (voices, frames) in [(1, 24), (10, 24), (10, 64)] {
        let mut instance = Instance::with_voices(8, voices);
        CYCLE_COUNTER.with(|counter| counter.set(u32::MAX - 50));
        let mut samples = std::vec![0.0; LAST_BUS * frames];
        for _ in 0..256 {
            instance.render(&mut samples);
        }
        let expected = frames.div_ceil(levels::CHUNK) as u32 * 100;
        let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
        assert_eq!(context.state().profiling.render_cycles.average, expected);
        instance.draw();
        TEXT.with(|text| {
            let text = text.borrow();
            assert!(text.iter().any(|(_, _, value)| value == "Push: Mode"));
            assert_eq!(
                text.last(),
                Some(&(2, 62, std::format!("Render cyc: {expected}")))
            );
        });
        FOOTER_COVER_AFTER_HELP.with(|covered| assert!(covered.get()));
        SHAPES.with(|shapes| {
            assert_eq!(
                shapes.borrow().last(),
                Some(&(SHAPE_RECTANGLE, 0, 50, 255, 63))
            )
        });
        instance.change(LEFT_OUTPUT, 0);
        instance.change(RIGHT_OUTPUT, 0);
        for _ in 0..256 {
            instance.render(&mut samples);
        }
        let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
        assert_eq!(context.state().profiling.render_cycles.average, 0);
    }
}

#[no_mangle]
unsafe extern "C" fn NT_sendMidiSysEx(destination: u32, data: *const u8, count: u32, end: bool) {
    assert_eq!(destination, DESTINATION_USB);
    assert!(end);
    let bytes = unsafe { core::slice::from_raw_parts(data, count as usize) };
    SENT_SYSEX.with(|messages| messages.borrow_mut().push(bytes.to_vec()));
}

#[cfg(feature = "profiling")]
#[test]
fn profiling_stream_is_opt_in_usb_only_and_preserves_instance_identity() {
    let mut instance = Instance::new(8);
    let mut context = unsafe { Callback::<Audio>::from_raw(instance.base) };
    assert!(!context.send_midi_sysex(DESTINATION_USB, &[]));
    assert!(!context.send_midi_sysex(DESTINATION_USB, &[0xf0, 0x80]));
    assert!(!context.send_midi_sysex(0, &[0xf0, 0x7d]));
    SENT_SYSEX.with(|messages| messages.borrow_mut().clear());
    unsafe {
        factory().midi_sysex.unwrap()([0xf0, 0x7d, b'W', b'S', 1, 0x10, 0xf7].as_ptr(), 7);
    }
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..2000 {
        instance.render(&mut samples);
    }
    unsafe {
        factory().midi_sysex.unwrap()([0xf0, 0x7d, b'W', b'S', 1, 0x11, 0xf7].as_ptr(), 7);
    }
    SENT_SYSEX.with(|messages| {
        let messages = messages.borrow();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].len(), telemetry::PACKET_LEN);
        assert_eq!(&messages[0][6..11], &[0, 0, 0, 0, 0]);
    });
    for _ in 0..2000 {
        instance.render(&mut samples);
    }
    SENT_SYSEX.with(|messages| assert_eq!(messages.borrow().len(), 1));
}

#[test]
fn shared_cv_conversion_handles_detuning_distinct_inputs_rerouting_and_exact_jitter() {
    let mut instance = Instance::with_voices(8, 10);
    instance.change(PITCH, 69);
    for voice in 0..10 {
        instance.change(voice_pitch_parameter(voice), 1);
        instance.change(voice_fine_parameter(voice), voice as i16 * 10 - 50);
        instance.change(voice_transpose_parameter(voice), voice as i16 - 5);
    }
    let mut samples = [0.0; LAST_BUS * 24];
    for voltage in [0.125f32, 0.125001, -0.25, 1.1] {
        samples[..24].fill(voltage);
        let before = dsp::playback_work_counts().1;
        instance.render(&mut samples);
        let calls = dsp::playback_work_counts().1 - before;
        assert!(calls <= if voltage == 0.125 { 11 } else { 1 });
        for voice in 0..10 {
            let octave =
                (voice as f32 - 5.0) / 12.0 + (voice as f32 * 10.0 - 50.0) / 1200.0 + voltage;
            assert_cached_octave(&instance, voice, octave);
        }
    }
    for voice in 0..10 {
        instance.change(voice_pitch_parameter(voice), voice as i16 + 1);
        samples[voice * 24..(voice + 1) * 24].fill(voice as f32 * 0.1);
    }
    instance.render(&mut samples);
    for voice in 0..10 {
        let octave = (voice as f32 - 5.0) / 12.0
            + (voice as f32 * 10.0 - 50.0) / 1200.0
            + voice as f32 * 0.1;
        assert_cached_octave(&instance, voice, octave);
    }
    instance.change(voice_pitch_parameter(1), 10);
    instance.render(&mut samples);
    assert_cached_octave(&instance, 1, -4.0 / 12.0 - 40.0 / 1200.0 + 0.9);
}

#[cfg(not(feature = "profiling"))]
#[test]
fn normal_build_preserves_help_without_cycle_reads_or_profiling_sysex() {
    let mut instance = Instance::with_voices(8, 10);
    assert!(unsafe { factory().midi_sysex.is_none() });
    CYCLE_COUNTER.with(|counter| counter.set(12345));
    SENT_SYSEX.with(|messages| messages.borrow_mut().clear());
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..2000 {
        instance.render(&mut samples);
    }
    instance.draw();
    CYCLE_COUNTER.with(|counter| assert_eq!(counter.get(), 12345));
    SENT_SYSEX.with(|messages| assert!(messages.borrow().is_empty()));
    TEXT.with(|text| {
        let text = text.borrow();
        assert!(text.iter().any(|(_, _, value)| value == "Turn: Select"));
        assert_eq!(text.last(), Some(&(159, 64, String::from("Push: Mode"))));
        assert!(!text
            .iter()
            .any(|(_, _, value)| value.starts_with("Render cyc:")));
    });
    FOOTER_COVER_AFTER_HELP.with(|covered| assert!(!covered.get()));
}

#[test]
fn grouped_layout_has_none_and_ten_individual_defaults_then_none() {
    assert_eq!(MAX_VOICES, 24);
    assert_eq!(midi::GROUP_COUNT, 10);
    for segments in [2, 8, 32] {
        let instance = Instance::configured(segments, MAX_VOICES as i32);
        let config = Wavesmith::configuration(&[MAX_VOICES as i32, segments]).unwrap();
        assert_eq!(config.pages, 29);
        assert_eq!(config.parameters, 205);
        assert!(config.parameters <= 256 && config.pages <= 31);
        for voice in 0..MAX_VOICES {
            let group = voice_midi_group_parameter(voice);
            let d = unsafe { &*(*instance.base).parameters.add(group) };
            assert_eq!(unsafe { CStr::from_ptr(d.name) }, c"MIDI Voice Group");
            assert_eq!(
                (d.min, d.max, d.default, d.unit),
                (0, 10, default_group_value(voice), UNIT_ENUM)
            );
            assert_eq!(
                parameter_prefix(&instance, group as i32),
                std::format!("V{}:", voice + 1)
            );
            assert_eq!(instance.values[group], default_group_value(voice));
        }
    }
}

#[test]
fn unequal_groups_skip_empty_slots_and_share_notes_bend_and_release() {
    let mut instance = Instance::with_voices(8, 7);
    instance.change(MIDI_RELEASE, 25); // Explicit timing fixture, independent of defaults.
    instance.change(MIDI_CHANNEL, 1);
    instance.change(OUTPUT_MODE, 1);
    instance.change(RIGHT_OUTPUT, 14);
    for voice in 0..7 {
        instance.change(
            voice_midi_group_parameter(voice),
            if voice < 5 { 1 } else { 4 },
        );
        instance.change(voice_transpose_parameter(voice), voice as i16 - 3);
        instance.change(voice_fine_parameter(voice), voice as i16 * 5 - 15);
        instance.change(voice_pan_parameter(voice), voice as i16 * 16);
    }
    instance.midi(0x90, 69, 100);
    instance.midi(0x90, 76, 100);
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..300 {
        instance.render(&mut samples);
    }
    let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
    assert_eq!(context.state().midi.groups[0].pitch(), 69);
    assert_eq!(context.state().midi.groups[3].pitch(), 76);
    assert!(context
        .state()
        .midi
        .groups
        .iter()
        .enumerate()
        .all(|(g, n)| n.sounding() == (g == 0 || g == 3)));
    for voice in 0..7 {
        let octave = if voice < 5 { 0.0 } else { 7.0 / 12.0 }
            + (voice as f32 - 3.0) / 12.0
            + (voice as f32 * 5.0 - 15.0) / 1200.0;
        assert_cached_octave(&instance, voice, octave);
    }
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - before, 7 * 24);
    instance.midi(0xe0, 0, 96);
    instance.render(&mut samples);
    for voice in 0..7 {
        let octave = if voice < 5 { 0.0 } else { 7.0 / 12.0 }
            + (voice as f32 - 3.0) / 12.0
            + (voice as f32 * 5.0 - 15.0) / 1200.0
            + 1.0 / 12.0;
        assert_cached_octave(&instance, voice, octave);
    }
    instance.midi(0x80, 69, 0);
    for _ in 0..50 {
        instance.render(&mut samples);
    }
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - before, 2 * 24);
    instance.midi(0x80, 76, 0);
    for _ in 0..51 {
        instance.render(&mut samples);
    }
    assert!(samples[12 * 24..14 * 24].iter().all(|&v| v == 0.0));
}

#[test]
fn shared_group_gates_advance_once_even_with_five_and_two_members() {
    let mut instance = Instance::with_voices(8, 7);
    instance.change(MIDI_RELEASE, 25); // Explicit timing fixture, independent of defaults.
    instance.change(MIDI_CHANNEL, 1);
    instance.change(OUTPUT_MODE, 1);
    for voice in 0..7 {
        instance.change(
            voice_midi_group_parameter(voice),
            if voice < 5 { 1 } else { 4 },
        );
    }
    let mut enabled = [false; midi::GROUP_COUNT];
    enabled[0] = true;
    enabled[3] = true;
    let mut expected = midi::MidiState::new(48000);
    expected.set_envelope(5, 25);
    expected.set_channel(1);
    let mut samples = [0.0; LAST_BUS * 24];
    for block in 0..125 {
        let event = match block {
            0 => Some((0x90, 60, 100)),
            1 => Some((0x90, 64, 100)),
            20 => Some((0x90, 67, 100)),
            21 => Some((0x80, 60, 0)),
            60 => Some((0x80, 64, 0)),
            65 => Some((0x80, 67, 0)),
            _ => None,
        };
        if let Some((status, note, velocity)) = event {
            instance.midi(status, note, velocity);
            expected.message(status, note, velocity, &enabled);
        }
        expected.begin_block();
        for group in &mut expected.groups {
            group.take_retune();
        }
        let mut expected_ticks = 0;
        for _ in 0..24 {
            for (index, group) in expected.groups.iter_mut().enumerate() {
                if group.next_gain(expected.attack_step, expected.gate_step) > 0.0 {
                    expected_ticks += if index == 0 {
                        5
                    } else if index == 3 {
                        2
                    } else {
                        0
                    };
                }
            }
        }
        let before = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(
            dsp::oscillator_tick_count() - before,
            expected_ticks,
            "group gate cache must not replay a release in a later chunk, block {block}"
        );
        let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
        assert_eq!(context.state().midi, expected, "block {block}");
    }
    assert!(samples[12 * 24..13 * 24].iter().all(|&v| v == 0.0));
}

#[test]
fn live_group_reassignment_and_none_changes_leave_other_members_playing() {
    let mut instance = Instance::with_voices(8, 3);
    instance.change(MIDI_RELEASE, 25); // Explicit timing fixture, independent of defaults.
    instance.change(MIDI_CHANNEL, 1);
    instance.change(OUTPUT_MODE, 1);
    instance.change(voice_midi_group_parameter(2), 1);
    instance.midi(0x90, 60, 100);
    instance.midi(0x90, 67, 100);
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..300 {
        instance.render(&mut samples);
    }
    assert_cached_octave(&instance, 2, (60.0 - 69.0) / 12.0);
    instance.change(voice_midi_group_parameter(2), 2);
    instance.render(&mut samples);
    let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
    assert_eq!(
        context.state().voice_groups[2],
        0,
        "fade old group before adopting new pitch"
    );
    for _ in 0..50 {
        instance.render(&mut samples);
    }
    let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
    assert_eq!(context.state().voice_groups[2], 1);
    assert_eq!(context.state().midi.groups[0].pitch(), 60);
    assert_eq!(context.state().midi.groups[1].pitch(), 67);
    assert_cached_octave(&instance, 2, (67.0 - 69.0) / 12.0);
    instance.change(voice_midi_group_parameter(2), 0);
    for _ in 0..50 {
        instance.render(&mut samples);
    }
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - before, 2 * 24);
    let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
    assert!(context.state().midi.groups[1].sounding());
    instance.change(voice_midi_group_parameter(2), 2);
    for _ in 0..50 {
        instance.render(&mut samples);
    }
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - before, 3 * 24);
    instance.change(voice_midi_group_parameter(1), 0);
    instance.change(voice_midi_group_parameter(2), 0);
    instance.midi(0x90, 72, 100);
    for _ in 0..50 {
        instance.render(&mut samples);
    }
    let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
    assert_eq!(context.state().midi.groups[0].pitch(), 72);
    assert!(!context.state().midi.groups[1].sounding());
}

#[test]
fn twenty_four_voices_use_ten_midi_notes_and_steal_an_entire_oldest_group() {
    let mut instance = Instance::with_voices(32, 24);
    instance.change(MIDI_CHANNEL, 1);
    instance.change(OUTPUT_MODE, 1);
    instance.change(RIGHT_OUTPUT, 14);
    for voice in 0..24 {
        instance.change(
            voice_midi_group_parameter(voice),
            (voice % midi::GROUP_COUNT) as i16 + 1,
        );
        instance.change(voice_fine_parameter(voice), voice as i16 * 4 - 46);
        instance.change(
            voice_pan_parameter(voice),
            if voice % 2 == 0 { 0 } else { 100 },
        );
    }
    for note in 60..70 {
        instance.midi(0x90, note, 100);
    }
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..400 {
        instance.render(&mut samples);
    }
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - before, 24 * 24);
    instance.midi(0x90, 72, 100);
    instance.midi(0x80, 60, 0);
    for _ in 0..50 {
        instance.render(&mut samples);
    }
    for voice in 0..24 {
        let group = voice % midi::GROUP_COUNT;
        let note = if group == 0 { 72 } else { 60 + group };
        assert_cached_octave(
            &instance,
            voice,
            (note as f32 - 69.0) / 12.0 + (voice as f32 * 4.0 - 46.0) / 1200.0,
        );
    }
    assert!(samples[12 * 24..14 * 24]
        .iter()
        .all(|v| v.is_finite() && v.abs() <= 5.001));
    instance.midi(0xb0, 120, 0);
    instance.render(&mut samples);
    assert!(samples[12 * 24..14 * 24].iter().all(|&v| v == 0.0));
}

#[test]
fn group_parameter_does_not_change_cv_audio_and_only_active_groups_allocate() {
    let mut a = Instance::with_voices(8, 24);
    let mut b = Instance::with_voices(8, 24);
    a.change(OUTPUT_MODE, 1);
    b.change(OUTPUT_MODE, 1);
    for voice in 0..24 {
        b.change(voice_midi_group_parameter(voice), 10 - (voice % 10) as i16);
    }
    let mut x = [0.0; LAST_BUS * 24];
    let mut y = x;
    for _ in 0..200 {
        a.render(&mut x);
        b.render(&mut y);
        assert_eq!(x, y);
    }
    let mut instance = Instance::configured(8, 24);
    for voice in 0..24 {
        instance.change(voice_midi_group_parameter(voice), 0);
    }
    instance.change(MIDI_CHANNEL, 1);
    instance.change(OUTPUT_MODE, 1);
    instance.midi(0x90, 60, 100);
    let mut samples = [0.0; LAST_BUS * 24];
    instance.render(&mut samples);
    let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
    assert!(context.state().midi.groups.iter().all(|g| !g.sounding()));
    instance.change(voice_midi_group_parameter(23), 10);
    instance.midi(0x90, 67, 100);
    for _ in 0..300 {
        instance.render(&mut samples);
    }
    let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
    assert!(context
        .state()
        .midi
        .groups
        .iter()
        .enumerate()
        .all(|(g, n)| n.sounding() == (g == 9)));
    assert_cached_octave(&instance, 23, (67.0 - 69.0) / 12.0);
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - before, 24);
}

#[test]
fn voices_specification_creates_only_requested_parameters_pages_and_processing_slots() {
    let spec = &Wavesmith::SPECIFICATIONS[0];
    assert_eq!(
        (spec.name, spec.min, spec.max, spec.default),
        (c"Voices" as &CStr, 1, 24, 1)
    );
    for voices in 1..=MAX_VOICES {
        let mut instance = Instance::with_specified_voices(voices as i32);
        let config = Wavesmith::configuration(&[voices as i32, 8]).unwrap();
        assert_eq!(
            (config.parameters, config.pages),
            (85 + voices * 5, 5 + voices)
        );
        assert_eq!(instance.values.len(), config.parameters);
        assert_eq!(Wavesmith::SPECIFICATIONS[1].name, c"Segments");
        assert_eq!(Wavesmith::SPECIFICATIONS[1].default, 32);
        let mut samples = [0.0; LAST_BUS * 24];
        GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
        for _ in 0..200 {
            instance.render(&mut samples);
        }
        let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
        assert_eq!(context.state().voices, voices);
        assert!(context.state().grayed_for_midi.is_some());
        assert!(context.state().grayed_for_stereo.is_some());
        assert!(context.state().waveform_grayed);
        GRAY_CALLS
            .with(|calls| assert!(calls.borrow().iter().all(|&(p, _)| p < config.parameters)));
        GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
        let before = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count() - before, voices * 24);
        GRAY_CALLS.with(|calls| assert!(calls.borrow().is_empty()));
        assert!(samples[12 * 24..13 * 24].iter().any(|x| x.abs() > 0.01));
        assert_eq!(
            parameter_prefix(&instance, voice_pitch_parameter(voices) as i32),
            ""
        );
        instance.change(MIDI_CHANNEL, 1);
        for voice in 0..voices {
            instance.change(
                voice_midi_group_parameter(voice),
                (voice % midi::GROUP_COUNT + 1) as i16,
            );
        }
        for group in 0..voices.min(midi::GROUP_COUNT) {
            instance.midi(0x90, 60 + group as u8, 100);
        }
        for _ in 0..200 {
            instance.render(&mut samples);
        }
        let before = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count() - before, voices * 24);
        for voice in 0..voices {
            assert_cached_octave(
                &instance,
                voice,
                (60.0 + (voice % midi::GROUP_COUNT) as f32 - 69.0) / 12.0,
            );
        }
        instance.midi(0xb0, 120, 0);
        instance.render(&mut samples);
        // Add mode preserves previous bus samples, so verify no oscillator work.
        let before = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count(), before);
    }
    for invalid in [-1, 0, 25, 32] {
        assert!(Wavesmith::configuration(&[invalid, 8]).is_none());
    }
}

#[test]
fn segment_specification_hides_unused_rows_but_keeps_them_allocated_and_grayed() {
    for segments in 2..=32 {
        for voices in [1, 3, 24] {
            let mut instance = Instance::configured(segments, voices);
            let config = Wavesmith::configuration(&[voices, segments]).unwrap();
            assert_eq!(config.parameters, 85 + voices as usize * 5);
            assert_eq!(
                config.page_indices,
                GLOBALS + 2 + voices as usize * 5 + segments as usize * 2 + 1
            );
            let pages = unsafe { &*(*instance.base).parameter_pages };
            let waveform = unsafe { &*(*pages).pages.add(voices as usize + 4) };
            let rows = unsafe {
                core::slice::from_raw_parts(waveform.params, waveform.num_params as usize)
            };
            assert_eq!(rows.len(), 2 * segments as usize + 1);
            assert_eq!(rows.last().copied(), Some(WAVEFORM_START as u8));
            for i in 0..32 {
                assert_eq!(
                    rows.contains(&(curve_parameter(i) as u8)),
                    i < segments as usize
                );
                assert_eq!(
                    rows.contains(&(boundary_parameter(i) as u8)),
                    i < segments as usize
                );
                let curve = unsafe { &*(*instance.base).parameters.add(curve_parameter(i)) };
                assert!(
                    !curve.name.is_null(),
                    "hidden slots still need valid metadata"
                );
            }
            unsafe { Callback::<Audio>::from_raw(instance.base) }
                .state_mut()
                .waveform_grayed = false;
            GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
            instance.render(&mut [0.0; LAST_BUS * 24]);
            GRAY_CALLS.with(|calls| {
                let calls = calls.borrow();
                for i in 0..32 {
                    assert!(calls.contains(&(curve_parameter(i), i >= segments as usize)));
                    assert!(calls.contains(&(boundary_parameter(i), i >= segments as usize)));
                }
            });
            GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
            instance.render(&mut [0.0; LAST_BUS * 24]);
            GRAY_CALLS.with(|calls| assert!(calls.borrow().is_empty()));
        }
    }
}

#[test]
fn both_specifications_preserve_all_surviving_ordinals_values_and_waveform_tail() {
    for old_segments in 2..=32 {
        let mut old = Instance::configured(old_segments, 3);
        for (i, value) in old.values.iter_mut().enumerate() {
            let d = unsafe { &*(*old.base).parameters.add(i) };
            *value = d.min + (i % (d.max as i32 - d.min as i32 + 1) as usize) as i16;
        }
        for new_segments in 2..=32 {
            for voices in [1, 3, 24] {
                let mut new = Instance::configured(new_segments, voices);
                let retained = old.values.len().min(new.values.len());
                new.values[..retained].copy_from_slice(&old.values[..retained]);
                for i in 0..retained {
                    let a = unsafe { CStr::from_ptr((*(*old.base).parameters.add(i)).name) };
                    let b = unsafe { CStr::from_ptr((*(*new.base).parameters.add(i)).name) };
                    assert_eq!(
                        a, b,
                        "segments {old_segments}->{new_segments}, voices {voices}, parameter {i}"
                    );
                    assert_eq!(
                        parameter_prefix(&old, i as i32),
                        parameter_prefix(&new, i as i32)
                    );
                    assert_eq!(old.values[i], new.values[i]);
                }
                assert_eq!(
                    &old.values[WAVEFORM_START..VOICE_START],
                    &new.values[WAVEFORM_START..VOICE_START]
                );
                let wave = read_wave(new_segments as usize, |p| new.values.get(p).copied());
                assert_eq!(
                    wave.segment_sample(new_segments as usize - 1, 1.0),
                    wave.segment_sample(0, 0.0)
                );
            }
        }
    }
}

#[test]
fn hidden_wave_parameters_do_not_change_audio_and_return_after_respecification() {
    let mut a = Instance::configured(2, 1);
    let mut b = Instance::configured(2, 1);
    a.change(OUTPUT_MODE, 1);
    b.change(OUTPUT_MODE, 1);
    let mut x = [0.0; LAST_BUS * 24];
    let mut y = x;
    for _ in 0..300 {
        a.render(&mut x);
        b.render(&mut y);
        assert_eq!(x, y);
    }
    a.change(boundary_parameter(31), -90);
    a.change(curve_parameter(31), 85);
    for _ in 0..200 {
        a.render(&mut x);
        b.render(&mut y);
        assert_eq!(x, y);
    }
    let mut expanded = Instance::configured(32, 1);
    expanded.values.copy_from_slice(&a.values);
    let wave = read_wave(32, |p| expanded.values.get(p).copied());
    assert_eq!(wave.boundaries[31], -90);
    assert_eq!(wave.curves[31], 85);
    for _ in 0..300 {
        expanded.render(&mut x);
    }
    assert!(x.iter().all(|x| x.is_finite()));
}

#[test]
fn both_specification_ranges_are_validated_and_no_segments_parameter_remains() {
    assert_eq!(Wavesmith::SPECIFICATIONS.len(), 2);
    let a = &Wavesmith::SPECIFICATIONS[0];
    let b = &Wavesmith::SPECIFICATIONS[1];
    assert_eq!(
        (a.name, a.min, a.max, a.default),
        (c"Voices" as &CStr, 1, 24, 1)
    );
    assert_eq!(
        (b.name, b.min, b.max, b.default),
        (c"Segments" as &CStr, 2, 32, 32)
    );
    for invalid in [
        std::vec![],
        std::vec![1],
        std::vec![0, 8],
        std::vec![25, 8],
        std::vec![1, 1],
        std::vec![1, 33],
    ] {
        assert!(Wavesmith::configuration(&invalid).is_none());
    }
    let instance = Instance::with_specified_voices(1);
    for i in 0..instance.values.len() {
        assert_ne!(
            unsafe { CStr::from_ptr((*(*instance.base).parameters.add(i)).name) },
            c"Segments"
        );
    }
}

#[test]
fn none_groups_do_not_mute_any_specified_cv_voices_but_disable_midi_allocation() {
    for voices in [1, 10, 24] {
        let mut instance = Instance::configured(8, voices);
        for voice in 0..voices as usize {
            instance.change(voice_midi_group_parameter(voice), 0);
        }
        let mut samples = [0.0; LAST_BUS * 24];
        for _ in 0..200 {
            instance.render(&mut samples);
        }
        let before = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count() - before, voices as usize * 24);
        instance.change(MIDI_CHANNEL, 1);
        for note in 60..72 {
            instance.midi(0x90, note, 100);
        }
        for _ in 0..200 {
            instance.render(&mut samples);
        }
        let before = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count(), before);
        let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
        assert!(context
            .state()
            .midi
            .groups
            .iter()
            .all(|group| !group.sounding()));
        instance.change(MIDI_CHANNEL, 0);
        for _ in 0..200 {
            instance.render(&mut samples);
        }
        let before = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count() - before, voices as usize * 24);
        instance.assert_guard();
    }
}

#[test]
fn factory_defaults_allocate_ten_notes_and_later_voices_join_only_when_assigned() {
    let mut instance = Instance::configured(8, 24);
    for voice in 0..24 {
        assert_eq!(
            instance.values[voice_midi_group_parameter(voice)],
            if voice < 10 { voice as i16 + 1 } else { 0 }
        );
    }
    instance.change(MIDI_CHANNEL, 1);
    for note in 60..70 {
        instance.midi(0x90, note, 100);
    }
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..200 {
        instance.render(&mut samples);
    }
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - before, 10 * 24);
    instance.change(voice_midi_group_parameter(10), 10);
    for _ in 0..200 {
        instance.render(&mut samples);
    }
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - before, 11 * 24);
    assert_cached_octave(&instance, 10, 0.0); // Group 10 holds A4 (69).
    instance.change(voice_midi_group_parameter(10), 0);
    for _ in 0..200 {
        instance.render(&mut samples);
    }
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - before, 10 * 24);
    let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
    assert_eq!(context.state().voice_gains[10], 0.0);
    assert_eq!(context.state().voice_groups[10], midi::NO_GROUP);
    instance.assert_guard();
}

#[test]
fn midi_tuning_setup_only_runs_for_sounding_voices_including_release_tails() {
    let mut instance = Instance::configured(8, 24);
    instance.change(MIDI_RELEASE, 25); // Explicit timing fixture, independent of defaults.
    instance.change(MIDI_CHANNEL, 1);
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..200 {
        instance.render(&mut samples);
    }
    MIDI_TUNING_READS.with(|reads| reads.set(0));
    instance.render(&mut samples);
    MIDI_TUNING_READS.with(|reads| assert_eq!(reads.get(), 0));
    instance.midi(0x90, 60, 100);
    instance.render(&mut samples);
    MIDI_TUNING_READS.with(|reads| assert_eq!(reads.get(), 1));
    for _ in 0..200 {
        instance.render(&mut samples);
    }
    instance.change(voice_transpose_parameter(0), 12);
    instance.change(voice_fine_parameter(0), 25);
    instance.render(&mut samples);
    assert_cached_octave(&instance, 0, (60.0 - 69.0 + 12.0) / 12.0 + 25.0 / 1200.0);
    instance.midi(0x80, 60, 0);
    MIDI_TUNING_READS.with(|reads| reads.set(0));
    instance.render(&mut samples);
    MIDI_TUNING_READS.with(|reads| assert_eq!(reads.get(), 1));
    for _ in 0..200 {
        instance.render(&mut samples);
    }
    MIDI_TUNING_READS.with(|reads| reads.set(0));
    instance.render(&mut samples);
    MIDI_TUNING_READS.with(|reads| assert_eq!(reads.get(), 0));
    instance.assert_guard();
}

#[test]
fn pot_reset_ignores_release_motion_for_100ms_then_resumes_from_current_position() {
    for mode in 0..3 {
        for pot in 0..3 {
            let mut instance = Instance::new(8);
            let mut pots = [0.5; 3];
            instance.event(pots, [0, 0]);
            for _ in 0..mode {
                instance.click_right(pots);
            }
            let parameter = match mode {
                0 => editor_parameter(8, 0, [0, 2, 1][pot]).unwrap(),
                1 => [WARP_POSITION, WARP_AMOUNT, SMOOTHING][pot],
                _ => [PITCH, FINE, LEVEL][pot],
            };
            let button = [POT_BUTTON_L, POT_BUTTON_C, POT_BUTTON_R][pot];
            let definition = unsafe { *(*instance.base).parameters.add(parameter) };
            let reset = if mode == 0 && pot != 1 {
                0
            } else {
                definition.default
            };
            instance.change(parameter, definition.max);
            instance.event_buttons(pots, [0, 0], button, 0);
            assert_eq!(instance.values[parameter], reset);
            let mut samples = [0.0; LAST_BUS * 24];
            // A long press must still get a fresh hold interval on release.
            for _ in 0..300 {
                instance.render(&mut samples);
            }
            pots[pot] = 0.505;
            instance.event_buttons(pots, [0, 0], 0, button);
            for _ in 0..198 {
                instance.render(&mut samples);
            } // 99 ms at 48 kHz.
            pots[pot] = 0.510;
            instance.event(pots, [0, 0]);
            assert_eq!(instance.values[parameter], reset);
            for _ in 0..2 {
                instance.render(&mut samples);
            }
            // Stationary after expiry: ignored movement must not be accumulated.
            instance.event(pots, [0, 0]);
            assert_eq!(instance.values[parameter], reset);
            // Resume normal pickup; move away from whichever rail is the default.
            let direction = if reset == definition.max { -1.0 } else { 1.0 };
            for _ in 0..8 {
                pots[pot] += direction * 0.005;
                instance.event(pots, [0, 0]);
            }
            assert_ne!(instance.values[parameter], reset);
        }
    }
}

#[test]
fn reshape_confirmation_labels_are_non_mutating_and_actions_reset_after_completion() {
    let mut instance = Instance::new(32);
    for trigger in SHAPE_TRIGGERS {
        let before = instance.values.clone();
        let seed = unsafe { Callback::<ReadOnly>::from_raw(instance.base) }
            .state()
            .random_seed;
        for (value, expected) in [(0, "Ready"), (1, "Apply")] {
            let mut buffer = [0 as c_char; 32];
            let length = unsafe {
                factory().parameter_string.unwrap()(
                    instance.base,
                    trigger as i32,
                    value,
                    buffer.as_mut_ptr(),
                )
            };
            assert_eq!(length as usize, expected.len());
            assert_eq!(
                unsafe { CStr::from_ptr(buffer.as_ptr()).to_str().unwrap() },
                expected
            );
        }
        // Previewing the proposed confirmed value must not perform the action.
        assert_eq!(instance.values, before);
        assert_eq!(
            unsafe { Callback::<ReadOnly>::from_raw(instance.base) }
                .state()
                .random_seed,
            seed
        );
        instance.change(trigger, 1);
        assert_eq!(instance.values[trigger], 0);
        assert!(
            !unsafe { Callback::<ReadOnly>::from_raw(instance.base) }
                .state()
                .trigger_high[SHAPE_TRIGGERS.iter().position(|&p| p == trigger).unwrap()]
        );
        let after = instance.values.clone();
        instance.change(trigger, 0);
        assert_eq!(instance.values, after);
    }
}

#[test]
fn envelope_metadata_is_midi_only_and_cv_audio_is_unchanged() {
    let mut a = Instance::new(8);
    let mut b = Instance::new(8);
    for (parameter, maximum, default) in [(MIDI_ATTACK, 500, 5), (MIDI_RELEASE, 1000, 500)] {
        let d = unsafe { &*(*a.base).parameters.add(parameter) };
        assert_eq!(
            (d.min, d.max, d.default, d.unit),
            (1, maximum, default, UNIT_MS)
        );
    }
    a.change(MIDI_ATTACK, 500);
    a.change(MIDI_RELEASE, 1000);
    let mut x = [0.0; LAST_BUS * 24];
    let mut y = x;
    for _ in 0..300 {
        x.fill(0.0);
        y.fill(0.0);
        a.render(&mut x);
        b.render(&mut y);
        assert_eq!(x, y);
    }
}

#[test]
fn restored_envelope_values_apply_before_the_first_midi_note_and_release_tail_stops() {
    let mut instance = Instance::new(8);
    instance.values[MIDI_CHANNEL] = 1;
    instance.values[MIDI_ATTACK] = 500;
    instance.values[MIDI_RELEASE] = 1000;
    instance.change(OUTPUT_MODE, 1);
    instance.midi(0x90, 60, 100);
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..1100 {
        instance.render(&mut samples);
    }
    let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
    assert_eq!(
        context.state().midi.groups[0].gate_state(),
        midi::GateState::Steady
    );
    instance.midi(0x80, 60, 0);
    for _ in 0..1900 {
        instance.render(&mut samples);
    }
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert!(dsp::oscillator_tick_count() > before);
    for _ in 0..105 {
        instance.render(&mut samples);
    }
    let before = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count(), before);
    assert!(samples[12 * 24..13 * 24].iter().all(|&v| v == 0.0));
}

#[test]
fn midi_single_note_release_is_audible_and_is_not_undone_by_level_control() {
    let mut instance = Instance::new(8);
    instance.change(MIDI_RELEASE, 25); // Explicit timing fixture, independent of defaults.
    instance.change(MIDI_CHANNEL, 1);
    instance.change(OUTPUT_MODE, 1);
    for i in 0..8 {
        instance.change(boundary_parameter(i), 80);
    }
    let mut samples = [0.0; LAST_BUS * 24];
    for _ in 0..300 {
        instance.render(&mut samples);
    }
    instance.midi(0x90, 60, 100);
    for _ in 0..100 {
        instance.render(&mut samples);
    }
    let full = samples[13 * 24 - 1];
    assert!(full > 1.0);
    instance.midi(0x80, 60, 0);
    instance.render(&mut samples);
    assert!((samples[12 * 24] - full).abs() < full * 0.002);
    for _ in 1..25 {
        instance.render(&mut samples);
    }
    assert!((samples[13 * 24 - 1] / full - 0.5).abs() < 0.002);
    for _ in 0..26 {
        instance.render(&mut samples);
    }
    assert!(samples[12 * 24..13 * 24].iter().all(|&v| v == 0.0));
}

#[test]
fn midi_attacks_and_long_releases_use_batched_playback_for_all_group_members() {
    for voices in [1, 7] {
        let mut instance = Instance::with_voices(8, voices);
        for voice in 0..voices as usize {
            instance.change(voice_midi_group_parameter(voice), 1);
        }
        instance.change(OUTPUT_MODE, 1);
        instance.change(RIGHT_OUTPUT, 14);
        instance.change(MIDI_ATTACK, 500);
        instance.change(MIDI_RELEASE, 1000);
        instance.change(MIDI_CHANNEL, 1);
        let mut samples = [0.0; LAST_BUS * 24];
        for _ in 0..300 {
            instance.render(&mut samples);
        }
        instance.midi(0x90, 60, 100);
        // Let oscillator tuning and output-level slews settle during the slow attack.
        for _ in 0..100 {
            instance.render(&mut samples);
        }
        let before = dsp::playback_work_counts();
        instance.render(&mut samples);
        assert_eq!(
            dsp::playback_work_counts().0 - before.0,
            voices as usize * 24
        );
        assert_eq!(dsp::playback_work_counts().1, before.1);
        for _ in 0..1000 {
            instance.render(&mut samples);
        }
        instance.midi(0x80, 60, 0);
        let before = dsp::playback_work_counts();
        instance.render(&mut samples);
        assert_eq!(
            dsp::playback_work_counts().0 - before.0,
            voices as usize * 24
        );
        assert_eq!(dsp::playback_work_counts().1, before.1);
        assert!(samples[12 * 24..14 * 24].iter().any(|&v| v != 0.0));
        for _ in 0..2005 {
            instance.render(&mut samples);
        }
        let before = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count(), before);
        assert!(samples[12 * 24..14 * 24].iter().all(|&v| v == 0.0));
    }
}

#[test]
fn idle_ui_gate_tracks_unflagged_pots_before_pickup_resumes() {
    let mut instance = Instance::new(8);
    let boundary = editor_parameter(8, 0, 0).unwrap();
    instance.change(boundary, 0);
    instance.event([0.5; 3], [0, 0]);
    for step in 1..=100 {
        let data = NtUiData {
            pots: [0.5 + step as f32 * 0.0002, 0.5, 0.5],
            controls: 0,
            last_buttons: 0,
            encoders: [0; 2],
            unused: [0; 2],
        };
        unsafe {
            factory().custom_ui.unwrap()(instance.base, &data);
        }
        assert_eq!(instance.values[boundary], 0);
    }
    instance.event([0.525, 0.5, 0.5], [0, 0]);
    assert!(
        instance.values[boundary] > 0,
        "pickup must use the most recent idle sample"
    );
    instance.assert_guard();
}

#[test]
fn fresh_instances_use_sine_boundaries_and_zero_curves_for_every_segment_count() {
    for count in 2..=32 {
        let mut instance = Instance::new(count);
        let count = count as usize;
        let wave = dsp::Wave::new(count);
        for boundary in 0..count {
            let expected = (-100.0 * (std::f64::consts::TAU * boundary as f64 / count as f64).cos())
                .round() as i16;
            let parameter = boundary_parameter(boundary);
            let definition = unsafe { &*(*instance.base).parameters.add(parameter) };
            assert_eq!(definition.default, expected);
            assert_eq!(instance.values[parameter], expected);
            assert_eq!(wave.boundaries[boundary], expected);
            assert_eq!(instance.values[curve_parameter(boundary)], 0);
            assert_eq!(wave.curves[boundary], 0);
        }
        let original = instance.values.clone();
        instance.change(SHAPE_TRIGGERS[4], 1);
        assert_eq!(
            instance.values, original,
            "Sine action should match startup defaults"
        );
        assert_eq!(wave.sample(0.0), wave.sample(1.0));
        instance.assert_guard();
    }
}

#[test]
fn directly_initialized_subobjects_match_value_constructor_references() {
    for segments in 2..=32 {
        for voices in [1, 15, 24] {
            let instance = Instance::configured(segments, voices);
            let context = unsafe { Callback::<ReadOnly>::from_raw(instance.base) };
            let state = context.state();
            let rate = state.sample_rate as u32;
            assert_eq!(state.engine, dsp::Engine::new(segments as usize, rate));
            assert_eq!(state.midi, midi::MidiState::new(rate));
            assert_eq!(state.cv_pitch, dsp::CvPitchCache::new());
            instance.assert_guard();
        }
    }
}
