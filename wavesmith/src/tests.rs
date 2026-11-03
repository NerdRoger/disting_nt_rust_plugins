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
    editor_parameter(n, segment, field, 1).expect("invalid editor control")
}

struct Instance {
    base: *mut NtAlgorithm,
    layout: Layout,
    guard_offset: usize,
    voice_count: usize,
    values: Vec<i16>,
}

impl Instance {
    fn new(n: i32) -> Self {
        Self::with_voices(n, 1)
    }
    fn with_voices(n: i32, voices: i32) -> Self {
        unsafe {
            let specs = [n, voices];
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
                GLOBALS + n as usize * 2 + voices as usize * 3
            );
            assert!(
                req.sram
                    < (dsp::blocks(dsp::DEFAULT_TABLES) * core::mem::size_of::<dsp::Block>() + 8000)
                        as u32
            );
            assert!(core::mem::size_of::<State>() < 2048);
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
            let values: Vec<_> = (0..req.num_parameters as usize)
                .map(|i| (*(*base).parameters.add(i)).default)
                .collect();
            (*base).v = values.as_ptr();
            CURRENT.with(|current| current.set(base));
            CURRENT_SEGMENTS.with(|current| current.set(n as usize));
            GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
            Self {
                base,
                layout,
                guard_offset: req.sram as usize,
                voice_count: voices as usize,
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
        let instance = Instance::new(n);
        unsafe {
            assert_eq!((*(*instance.base).parameter_pages).num_pages as usize, 5);
            assert_eq!(
                editor_parameter(n as usize, 0, 0, 1),
                Some(seam_parameter(1))
            );
            assert_eq!(
                editor_parameter(n as usize, n as usize - 1, 1, 1),
                Some(seam_parameter(1))
            );
            let wave = read_wave(n as usize, |p| instance.values.get(p).copied(), 1);
            assert_eq!(wave.segment_sample(0, 0.0), 0.0);
            assert_eq!(wave.segment_sample(n as usize - 1, 1.0), 0.0);
            let pages = &*(*instance.base).parameter_pages;
            let oscillator = &*pages.pages;
            assert_eq!(CStr::from_ptr(oscillator.name), c"Oscillator");
            assert_eq!(
                core::slice::from_raw_parts(oscillator.params, oscillator.num_params as usize),
                &[MIDI_CHANNEL as u8, PITCH as u8, FINE as u8, LEVEL as u8]
            );
            let shape = &*pages.pages.add(2);
            assert_eq!(CStr::from_ptr(shape.name), c"Shape");
            let expected_names = [
                c"Warp Position",
                c"Warp Amount",
                c"Smoothing",
                c"Fold Limit",
                c"Sine Boundaries",
                c"Triangle Boundaries",
                c"Square Boundaries",
                c"Sawtooth Boundaries",
                c"Zero Curves",
                c"Randomize Curves",
                c"Randomize Boundaries",
                c"Randomize All",
            ];
            let rows = core::slice::from_raw_parts(shape.params, shape.num_params as usize);
            assert_eq!(rows.len(), expected_names.len());
            for (&index, name) in rows.iter().zip(expected_names) {
                assert_eq!(
                    CStr::from_ptr((*(*instance.base).parameters.add(index as usize)).name),
                    name
                );
            }
            let oscillator_group = (*pages.pages).group;
            let routing_group = (*pages.pages.add(1)).group;
            let page = &*pages.pages.add(4);
            assert_eq!(CStr::from_ptr(page.name), c"Waveform");
            assert_ne!(page.group, 0);
            assert_ne!(page.group, oscillator_group);
            assert_ne!(page.group, routing_group);
            let routing = &*pages.pages.add(1);
            assert_eq!(CStr::from_ptr(routing.name), c"Output");
            assert_eq!(
                core::slice::from_raw_parts(routing.params, routing.num_params as usize),
                &[OUTPUT as u8, OUTPUT_MODE as u8]
            );
            assert_eq!(instance.values[OUTPUT], 13);
            assert_eq!(instance.values[OUTPUT_MODE], 0);
            assert_eq!(instance.values[voice_pitch_parameter(0)], 0);
            assert_eq!(
                CStr::from_ptr((*(*instance.base).parameters.add(voice_pitch_parameter(0))).name),
                c"Pitch CV"
            );
            assert!(factory().parameter_ui_prefix.is_some());
            assert_eq!(page.num_params as usize, n as usize * 2 + 1);
            let rows = core::slice::from_raw_parts(page.params, page.num_params as usize);
            assert_eq!(rows[0] as usize, seam_parameter(1));
            for segment in 0..n as usize {
                assert_eq!(rows[segment * 2 + 1] as usize, curve_parameter(segment, 1));
                assert_eq!(
                    rows[segment * 2 + 2] as usize,
                    editor_parameter(n as usize, segment, 1, 1).unwrap_or(seam_parameter(1))
                );
            }
            let seam = *(*instance.base).parameters.add(seam_parameter(1));
            assert_eq!((seam.min, seam.max, seam.default), (-100, 100, 0));
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
                let p = boundary_parameter(boundary, 1);
                assert_eq!(references[p], 1);
                assert_eq!(editor_parameter(n as usize, boundary - 1, 1, 1), Some(p));
                assert_eq!(editor_parameter(n as usize, boundary, 0, 1), Some(p));
                let d = *(*instance.base).parameters.add(p);
                assert_eq!((d.min, d.max), (-100, 100));
                assert_eq!(
                    CStr::from_ptr(d.name).to_str().unwrap(),
                    std::format!("Boundary {boundary:02}")
                );
            }
            for segment in 0..n as usize {
                assert_eq!(references[curve_parameter(segment, 1)], 1);
            }
            assert_eq!(references[seam_parameter(1)], 2);
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
            assert_eq!(fold, 9);
            assert_eq!(references[fold], 1);
            let d = *(*instance.base).parameters.add(fold);
            assert_eq!(CStr::from_ptr(d.name), c"Fold Limit");
            assert_eq!((d.min, d.max, d.default, d.unit), (0, 2, 2, UNIT_NONE));
            for (index, name) in SHAPE_TRIGGERS.into_iter().zip([
                c"Randomize All",
                c"Randomize Boundaries",
                c"Randomize Curves",
                c"Zero Curves",
                c"Sine Boundaries",
                c"Triangle Boundaries",
                c"Square Boundaries",
                c"Sawtooth Boundaries",
            ]) {
                assert_eq!(references[index], 1);
                let d = *(*instance.base).parameters.add(index);
                assert_eq!(CStr::from_ptr(d.name), name);
                assert_eq!((d.min, d.max, d.default, d.unit), (0, 1, 0, UNIT_NONE));
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
    for n in [0, 1, 33, -1] {
        assert!(Wavesmith::configuration(&[n, 6]).is_none());
    }
    assert_eq!(pluginEntry(SELECTOR_VERSION, 0), API_VERSION_14);
}

#[test]
fn specification_layout_keeps_six_tables_fixed_and_defaults_to_one_voice() {
    assert_eq!(Wavesmith::SPECIFICATIONS.len(), 2);
    assert_eq!(Wavesmith::SPECIFICATIONS[0].name, c"Segments");
    assert_eq!(unsafe { factory() }.num_specifications, 2);
    for segments in 2..=32 {
        let mut instance = Instance::new(segments);
        assert_eq!(
            Wavesmith::configuration(&[segments, 1]).unwrap().regions,
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
            let mut boundaries = std::vec![seam_parameter(1)];
            boundaries.extend((1..n).map(|boundary| boundary_parameter(boundary, 1)));
            let curves: Vec<_> = (0..n).map(|segment| curve_parameter(segment, 1)).collect();
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
            instance.change(trigger, 1);
            assert_eq!(instance.values, after);
            instance.change(trigger, 0);
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
fn shape_trigger_edges_are_independent() {
    let mut instance = Instance::new(8);
    let [all, boundaries, curves, zero, ..] = SHAPE_TRIGGERS;
    for trigger in [all, boundaries, curves, zero] {
        instance.change(trigger, 1);
    }
    let boundary_values: Vec<_> = (1..8)
        .map(|i| instance.values[boundary_parameter(i, 1)])
        .collect();
    instance.change(curve_parameter(0, 1), 73);
    let before = instance.values.clone();
    for trigger in [all, boundaries, curves, zero] {
        instance.change(trigger, 1);
    }
    assert_eq!(instance.values, before);
    instance.change(curves, 0);
    instance.change(curves, 1);
    assert!((0..8).any(|i| instance.values[curve_parameter(i, 1)] != before[curve_parameter(i, 1)]));
    assert_eq!(
        (1..8)
            .map(|i| instance.values[boundary_parameter(i, 1)])
            .collect::<Vec<_>>(),
        boundary_values
    );
    instance.change(zero, 0);
    instance.change(zero, 1);
    assert!((0..8).all(|i| instance.values[curve_parameter(i, 1)] == 0));
}

#[test]
fn waveform_boundary_triggers_preserve_curves_globals_and_independent_edges() {
    for n in 2..=32 {
        let mut instance = Instance::new(n);
        let n = n as usize;
        instance.change(SMOOTHING, 43);
        instance.change(WARP_POSITION, 94);
        instance.change(WARP_AMOUNT, -68);
        for i in 0..n {
            instance.change(curve_parameter(i, 1), 20 + i as i16);
        }
        let boundaries: Vec<_> = std::iter::once(seam_parameter(1))
            .chain((1..n).map(|boundary| boundary_parameter(boundary, 1)))
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
            let trigger = SHAPE_TRIGGERS[4 + kind];
            let before = instance.values.clone();
            instance.change(trigger, 1);
            for (i, &parameter) in boundaries.iter().enumerate() {
                assert_eq!(
                    instance.values[parameter],
                    dsp::preset_boundary(shape, i, n)
                );
            }
            for parameter in 0..before.len() {
                if parameter != trigger && !boundaries.contains(&parameter) {
                    assert_eq!(instance.values[parameter], before[parameter]);
                }
            }
            // Holding this trigger high must not reapply the shape, even with
            // other waveform triggers already held high from earlier iterations.
            instance.change(seam_parameter(1), 73);
            let held = instance.values.clone();
            for trigger in SHAPE_TRIGGERS.into_iter().skip(4).take(kind + 1) {
                instance.change(trigger, 1);
            }
            assert_eq!(instance.values, held);
            instance.change(trigger, 0);
            instance.change(trigger, 1);
            assert_eq!(instance.values[seam_parameter(1)], -100);
        }
    }
}

#[test]
fn randomize_triggers_on_rising_edges_and_preserves_other_controls() {
    for n in 2..=32 {
        let mut instance = Instance::new(n);
        let n = n as usize;
        let trigger = RANDOMIZE_ALL;
        let shape_start = seam_parameter(1);
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
        instance.change(trigger, 1);
        assert_eq!(instance.values, first);
        instance.change(trigger, 0);
        assert_eq!(&instance.values[..trigger], &first[..trigger]);
        instance.change(trigger, 1);
        assert_ne!(
            &instance.values[shape_start..shape_end],
            &first[shape_start..shape_end]
        );
        let wave = read_wave(n, |p| instance.values.get(p).copied(), 1);
        let seam = instance.values[seam_parameter(1)] as f32 * 0.01;
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
        let seam = seam_parameter(1);
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
    let wave = read_wave(8, |p| instance.values.get(p).copied(), 1);
    assert_eq!(wave.segment_sample(2, 1.0), wave.segment_sample(3, 0.0));

    // Direct host/preset/modulation writes use that same single source of truth.
    instance.change(boundary, -83);
    let wave = read_wave(8, |p| instance.values.get(p).copied(), 1);
    assert_eq!(wave.segment_sample(2, 1.0), -0.83);
    assert_eq!(wave.segment_sample(3, 0.0), -0.83);
}

#[test]
fn every_supported_layout_is_continuous_after_boundary_and_curve_edits() {
    for n in 2..=32 {
        let mut instance = Instance::new(n);
        for boundary in 1..n as usize {
            instance.change(
                boundary_parameter(boundary, 1),
                (boundary * 37 % 201) as i16 - 100,
            );
        }
        for segment in 0..n as usize {
            instance.change(
                curve_parameter(segment, 1),
                if segment % 2 == 0 { -100 } else { 100 },
            );
        }
        let wave = read_wave(n as usize, |p| instance.values.get(p).copied(), 1);
        for boundary in 1..n as usize {
            assert_eq!(
                wave.segment_sample(boundary - 1, 1.0),
                wave.segment_sample(boundary, 0.0)
            );
        }
        assert_eq!(wave.segment_sample(0, 0.0), 0.0);
        assert_eq!(wave.segment_sample(n as usize - 1, 1.0), 0.0);
    }
}

#[test]
fn configurable_voices_have_shared_routing_and_grouped_voice_pages() {
    assert_eq!(
        (
            Wavesmith::SPECIFICATIONS[1].min,
            Wavesmith::SPECIFICATIONS[1].max,
            Wavesmith::SPECIFICATIONS[1].default
        ),
        (1, 10, 1)
    );
    for n in 2..=32 {
        for voices in 1..=VOICE_COUNT {
            let instance = Instance::with_voices(n, voices as i32);
            let config = Wavesmith::configuration(&[n, voices as i32]).unwrap();
            assert_eq!(config.pages, voices + 4);
            assert_eq!(
                config.regions * core::mem::size_of::<dsp::Block>(),
                24 * 1024
            );
            unsafe {
                let pages = &*(*instance.base).parameter_pages;
                let routing = &*pages.pages.add(1);
                let rows = core::slice::from_raw_parts(routing.params, routing.num_params as usize);
                assert_eq!(rows, &[OUTPUT as u8, OUTPUT_MODE as u8]);
                let group = (*pages.pages.add(3)).group;
                assert_ne!(group, 0);
                assert_ne!(group, routing.group);
                assert_ne!(group, (*pages.pages).group);
                assert_ne!(group, (*pages.pages.add(voices + 3)).group);
                let mut seen = std::vec![0; instance.values.len()];
                let mut next_ordinal = 0;
                for i in 0..pages.num_pages as usize {
                    let page = &*pages.pages.add(i);
                    for &p in core::slice::from_raw_parts(page.params, page.num_params as usize) {
                        if seen[p as usize] == 0 {
                            assert_eq!(
                                p as usize, next_ordinal,
                                "ordinal must match first appearance in menu order"
                            );
                            next_ordinal += 1;
                        }
                        seen[p as usize] += 1;
                    }
                }
                assert_eq!(next_ordinal, instance.values.len());
                for (p, references) in seen.into_iter().enumerate() {
                    assert_eq!(references, if p == seam_parameter(voices) { 2 } else { 1 });
                }
                for voice in 0..voices {
                    let page = &*pages.pages.add(voice + 3);
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
                        &[cv as u8, transpose as u8, fine as u8]
                    );
                    for (index, name, min, max, unit) in [
                        (cv, c"Pitch CV", 0, LAST_BUS as i16, UNIT_CV_INPUT),
                        (transpose, c"Transpose", -48, 48, UNIT_SEMITONES),
                        (fine, c"Fine tune", -100, 100, UNIT_CENTS),
                    ] {
                        let d = &*(*instance.base).parameters.add(index);
                        assert_eq!(CStr::from_ptr(d.name), name);
                        assert_eq!((d.min, d.max, d.default, d.unit), (min, max, 0, unit));
                        assert_eq!(
                            parameter_prefix(&instance, index as i32),
                            std::format!("V{}:", voice + 1)
                        );
                    }
                }
            }
        }
    }
    for invalid in [-1, 0, 11] {
        assert!(Wavesmith::configuration(&[8, invalid]).is_none());
    }
    assert!(Wavesmith::configuration(&[8]).is_none());
}

#[test]
fn stable_midi_and_cv_reuse_caches_for_only_the_configured_pool() {
    const FRAMES: usize = 24;
    for voices in 1..=VOICE_COUNT {
        let mut instance = Instance::with_voices(8, voices as i32);
        instance.change(OUTPUT_MODE, 1);
        for voice in 0..voices {
            instance.change(voice_pitch_parameter(voice), voice as i16 + 1);
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
    for voices in 1..=VOICE_COUNT {
        let mut instance = Instance::with_voices(8, voices as i32);
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
        for _ in 0..30 {
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
    for voices in 1..=VOICE_COUNT {
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
        instance.change(OUTPUT, 0);
        let before = dsp::oscillator_tick_count();
        samples.fill(2.0);
        for _ in 0..80 {
            instance.render(&mut samples);
        }
        assert_eq!(dsp::oscillator_tick_count(), before);
        assert!(samples.iter().all(|&v| v == 2.0));
        instance.change(OUTPUT, 13);
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count(), before);
        assert!(samples[12 * 24..13 * 24].iter().all(|&v| v == 0.0));
        instance.midi(0x90, 67, 100);
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count() - before, 24);
    }
}

#[test]
fn shared_cv_output_matches_sum_of_independent_voices_and_snapshots_inputs_first() {
    const FRAMES: usize = 24;
    for voices in 1..=VOICE_COUNT {
        let mut mixed = Instance::with_voices(8, voices as i32);
        mixed.change(OUTPUT, 1); // Overwrites Voice 1's pitch input after snapshot.
        mixed.change(OUTPUT_MODE, 1);
        for voice in 0..voices {
            mixed.change(voice_pitch_parameter(voice), voice as i16 + 1);
        }
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
            for (voice, mono) in references.iter_mut().enumerate() {
                let mut buffer = [0.0; LAST_BUS * FRAMES];
                buffer[..FRAMES].fill(voice as f32 * 0.25);
                mono.render(&mut buffer);
                for (sum, &v) in expected.iter_mut().zip(&buffer[12 * FRAMES..13 * FRAMES]) {
                    *sum += v;
                }
            }
            for (&actual, &expected) in samples[..FRAMES].iter().zip(&expected) {
                assert!(
                    (actual - expected).abs() < 0.00001,
                    "voices {voices}: {actual} vs {expected}"
                );
            }
        }
        let before = dsp::oscillator_tick_count();
        mixed.change(OUTPUT, 0);
        mixed.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count(), before);
    }
}

#[test]
fn configured_voices_sum_without_normalization_and_replace_clears_only_once() {
    let mut reference: Option<Vec<f32>> = None;
    for voices in 1..=VOICE_COUNT {
        let mut instance = Instance::with_voices(8, voices as i32);
        instance.change(OUTPUT_MODE, 1);
        let mut samples = [0.0; LAST_BUS * 64];
        for _ in 0..80 {
            samples.fill(2.0);
            instance.render(&mut samples);
        }
        let output = samples[12 * 64..13 * 64].to_vec();
        if let Some(mono) = &reference {
            for (&actual, &v) in output.iter().zip(mono) {
                assert!((actual - v * voices as f32).abs() < 0.0001);
            }
        } else {
            reference = Some(output);
        }
    }
}

#[test]
fn output_modes_routing_pitch_cv_and_live_shape_edits() {
    let mut instance = Instance::new(8);
    instance.change(OUTPUT, 64);
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
    instance.change(LEVEL, 0);
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
    instance.change(curve_parameter(2, 1), 100);
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
    instance.change(curve_parameter(0, 1), 100);
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
        let wave = read_wave(8, |p| instance.values.get(p).copied(), 1);
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
    instance.change(LEVEL, 35);
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
        for expected in ["Pitch A3", "Fine -25", "Level 35"] {
            assert!(text
                .iter()
                .any(|(_, y, value)| *y == 10 && value == expected));
        }
        let footer: Vec<_> = text
            .iter()
            .filter(|(_, y, _)| *y >= 50)
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
        (127, 100, 100)
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
        (0, -100, 0)
    );
    for step in 1..=100 {
        instance.event([step as f32 * 0.0002; 3], [0, 0]);
    }
    assert!(
        instance.values[PITCH] > 0 && instance.values[FINE] > -100 && instance.values[LEVEL] > 0
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
            instance.change(curve_parameter(segment, 1), segment as i16 * 3 - 50);
        }
        instance.event([0.5; 3], [0, 0]);
        let before = instance.values.clone();
        let mut selected = 0i32;
        for delta in [-1i8, 1, 127, -128, 1, -1] {
            selected = (selected + delta as i32).rem_euclid(n as i32);
            instance.event([0.508; 3], [delta, 0]);
            assert_eq!(instance.values, before);
            instance.draw();
            let expected = std::format!("Curve {}", before[curve_parameter(selected as usize, 1)]);
            TEXT.with(|text| {
                assert!(text
                    .borrow()
                    .iter()
                    .any(|(_, y, value)| *y == 10 && value == &expected))
            });
        }
        instance.event([0.508, 0.517, 0.508], [0, 0]);
        let edited = curve_parameter(selected as usize, 1);
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
            instance.change(editor_parameter(n, selected, 0, 1).unwrap(), -100);
            instance.change(curve_parameter(selected, 1), -37);
            instance.change(editor_parameter(n, selected, 1, 1).unwrap(), 100);
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
    instance.change(LEVEL, 30);
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
            .filter(|(_, y, _)| *y >= 50)
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
    assert_eq!(instance.values[LEVEL], 30);
    assert_eq!(instance.values[PITCH], 40);
    let first_shape = instance.values[seam_parameter(1)..].to_vec();
    assert_eq!(first_shape, before[seam_parameter(1)..]);
    instance.event([1.0; 3], [0, 0]);
    assert_eq!(&instance.values[seam_parameter(1)..], &first_shape);
    for step in (0..200).rev() {
        instance.event([step as f32 * 0.005; 3], [0, 0]);
    }
    assert_eq!(instance.values[SMOOTHING], 0);
    assert_eq!(instance.values[WARP_AMOUNT], -100);
    assert_eq!(instance.values[trigger], 0);
    assert_eq!(instance.values[WARP_POSITION], 0);
    assert_eq!(&instance.values[seam_parameter(1)..], &first_shape);
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
    assert_eq!(editor_parameter(32, 0, 0, 1), Some(seam_parameter(1)));
    assert!(instance.values[segment_parameter(0, 2)] > 0);
    assert_eq!(instance.values[SMOOTHING], 0);
    instance.event([0.51; 3], [31, 0]);
    let previous = instance.values[segment_parameter(31, 0)];
    instance.event([0.52; 3], [0, 0]);
    assert!(instance.values[segment_parameter(31, 0)] > previous);
    assert_eq!(editor_parameter(32, 31, 1, 1), Some(seam_parameter(1)));
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
        assert_eq!(text.len(), 5);
        assert!(text.iter().all(|(_, y, _)| *y != 57));
        for (x, prefix, field) in [(2, "Left", 0), (102, "Curve", 2), (190, "Right", 1)] {
            let value = instance.values[editor_parameter(32, 31, field, 1).unwrap()];
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
    } else {
        assert_eq!(size, TEXT_TINY);
    }
    let value = String::from(CStr::from_ptr(text).to_str().unwrap());
    TEXT.with(|text| text.borrow_mut().push((x, y, value)));
}
#[no_mangle]
unsafe extern "C" fn NT_drawShapeI(kind: u8, x0: i32, y0: i32, x1: i32, y1: i32, color: i32) {
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
fn shared_midi_output_matches_independent_notes_with_the_existing_gate_fades() {
    for voices in [1, 4, 10] {
        let mut mixed = Instance::with_voices(8, voices);
        mixed.change(OUTPUT_MODE, 1);
        mixed.change(MIDI_CHANNEL, 1);
        for voice in 0..voices {
            mixed.midi(0x90, 60 + voice as u8, 100);
        }
        let mut references: Vec<_> = (0..voices)
            .map(|voice| {
                let mut mono = Instance::new(8);
                mono.change(OUTPUT_MODE, 1);
                mono.change(MIDI_CHANNEL, 1);
                mono.midi(0x90, 60 + voice as u8, 100);
                mono
            })
            .collect();
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
            for mono in &mut references {
                let mut buffer = [0.0; LAST_BUS * 24];
                mono.render(&mut buffer);
                for (sum, &v) in expected.iter_mut().zip(&buffer[12 * 24..13 * 24]) {
                    *sum += v;
                }
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
        assert_eq!(pages.num_pages as usize, instance.voice_count + 4);
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
        instance.change(OUTPUT, 13);
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
                    curve_parameter(block / 16 % segments as usize, voices as usize),
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
        let voices = voices as usize;
        instance.change(PITCH, 69);
        instance.change(voice_transpose_parameter(0), 7);
        instance.change(voice_fine_parameter(0), -25);
        let original = instance.values.clone();
        let mut samples = [0.0; LAST_BUS * 24];
        instance.render(&mut samples);
        let mut expected = std::vec![(PITCH, false)];
        for voice in 0..voices {
            expected.push((voice_pitch_parameter(voice), false));
            expected.push((voice_transpose_parameter(voice), false));
        }
        GRAY_CALLS.with(|calls| assert_eq!(&*calls.borrow(), &expected));
        GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
        instance.render(&mut samples);
        GRAY_CALLS.with(|calls| assert!(calls.borrow().is_empty()));
        instance.change(MIDI_CHANNEL, 1);
        for item in &mut expected {
            item.1 = true;
        }
        GRAY_CALLS.with(|calls| assert_eq!(&*calls.borrow(), &expected));
        GRAY_CALLS.with(|calls| calls.borrow_mut().clear());
        instance.change(MIDI_CHANNEL, 4);
        instance.render(&mut samples);
        GRAY_CALLS.with(|calls| assert!(calls.borrow().is_empty()));
        instance.change(MIDI_CHANNEL, 0);
        for item in &mut expected {
            item.1 = false;
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
        assert_eq!(calls.len(), 21);
        assert!(calls.iter().all(|&(_, gray)| gray));
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
    assert!(instance.values[LEVEL] > 50);
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
    let actual = context.state().oscillators[voice].cached_frequency_for_test();
    let expected_hz = 440.0 * 2.0f32.powf(expected);
    assert!(
        (actual - expected_hz).abs() < expected_hz * 0.000001,
        "expected {expected_hz} Hz, got {actual}"
    );
}

#[test]
fn per_voice_tuning_adds_to_cv_root_and_fine_but_midi_ignores_transpose_and_cv() {
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
    assert_cached_octave(&instance, 0, 0.0);
    assert_cached_octave(&instance, 1, 1.0 + 50.0 / 1200.0);
    instance.change(voice_transpose_parameter(1), 48);
    samples[..48].fill(-5.0);
    instance.render(&mut samples);
    assert_cached_octave(&instance, 1, 1.0 + 50.0 / 1200.0);
    instance.change(voice_fine_parameter(1), -25);
    instance.render(&mut samples);
    assert_cached_octave(&instance, 1, 1.0);
}

#[test]
fn voice_prefixes_cover_only_voice_parameters_and_respect_small_buffers() {
    let instance = Instance::with_voices(8, 10);
    for parameter in [
        OUTPUT as i32,
        OUTPUT_MODE as i32,
        PITCH as i32,
        FINE as i32,
        MIDI_CHANNEL as i32,
        seam_parameter(10) as i32,
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
                        0 => {
                            editor_parameter(n, selected as usize, [0, 2, 1][pot], voices as usize)
                                .unwrap()
                        }
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
    instance.change(LEVEL, 80);
    instance.change(MIDI_CHANNEL, 1);
    instance.event([0.8; 3], [0, 0]);
    instance.click_right([0.8; 3]);
    instance.click_right([0.8; 3]);
    let buttons = POT_BUTTON_L | POT_BUTTON_C | POT_BUTTON_R;
    instance.event_buttons([0.8; 3], [0, 0], buttons, 0);
    assert_eq!(instance.values[PITCH], 100);
    assert_eq!(instance.values[FINE], 0);
    assert_eq!(instance.values[LEVEL], 50);
    instance.event_buttons([0.8; 3], [0, 0], 0, buttons);
    instance.change(MIDI_CHANNEL, 0);
    instance.event_buttons([0.8; 3], [0, 0], POT_BUTTON_L, 0);
    assert_eq!(instance.values[PITCH], 57);
}
