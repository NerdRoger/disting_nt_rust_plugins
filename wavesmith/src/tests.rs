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

// Resolve editor controls against the live instance's segment count.
fn segment_parameter(segment: usize, field: usize) -> usize {
    let n = CURRENT_SEGMENTS.with(|current| current.get());
    editor_parameter(n, segment, field).expect("invalid editor control")
}

struct Instance {
    base: *mut NtAlgorithm,
    layout: Layout,
    values: Vec<i16>,
}

impl Instance {
    fn new(n: i32) -> Self {
        unsafe {
            let specs = [n];
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
                GLOBALS + n as usize * 2 + 12 + (VOICE_COUNT - 1) * 3
            );
            assert!(
                req.sram
                    < (dsp::blocks(dsp::DEFAULT_TABLES) * core::mem::size_of::<dsp::Block>() + 8000)
                        as u32
            );
            assert!(core::mem::size_of::<State>() < 1024);
            let alignment =
                distingnt_api::api::plugin::Registration::<Wavesmith, 1>::alignment(&specs)
                    .unwrap();
            let layout = Layout::from_size_align(req.sram as usize, alignment).unwrap();
            let memory = alloc_zeroed(layout);
            assert!(!memory.is_null());
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
            Self {
                base,
                layout,
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
            assert_eq!(
                (*(*instance.base).parameter_pages).num_pages as usize,
                VOICE_COUNT + 2
            );
            assert_eq!(
                editor_parameter(n as usize, 0, 0),
                Some(seam_parameter(n as usize))
            );
            assert_eq!(
                editor_parameter(n as usize, n as usize - 1, 1),
                Some(seam_parameter(n as usize))
            );
            let wave = read_wave(n as usize, |p| instance.values.get(p).copied());
            assert_eq!(wave.segment_sample(0, 0.0), 0.0);
            assert_eq!(wave.segment_sample(n as usize - 1, 1.0), 0.0);
            let pages = &*(*instance.base).parameter_pages;
            let oscillator_group = (*pages.pages).group;
            let routing_group = (*pages.pages.add(1)).group;
            let page = &*pages.pages.add(VOICE_COUNT + 1);
            assert_eq!(CStr::from_ptr(page.name), c"Waveform");
            assert_ne!(page.group, 0);
            assert_ne!(page.group, oscillator_group);
            assert_ne!(page.group, routing_group);
            for (voice, name) in [c"Voice 1", c"Voice 2", c"Voice 3", c"Voice 4"]
                .into_iter()
                .enumerate()
            {
                let voice_page = &*pages.pages.add(voice + 1);
                assert_eq!(CStr::from_ptr(voice_page.name), name);
                assert_eq!(voice_page.group, routing_group);
                let indices =
                    core::slice::from_raw_parts(voice_page.params, voice_page.num_params as usize);
                let expected = voice_parameters(n as usize, voice).map(|p| p as u8);
                assert_eq!(indices, &expected);
                let [cv, output, mode] = voice_parameters(n as usize, voice);
                for (parameter, name) in [
                    (cv, c"Pitch CV"),
                    (output, c"Output"),
                    (mode, c"Output mode"),
                ] {
                    let d = *(*instance.base).parameters.add(parameter);
                    assert_eq!(d.min, 0);
                    assert_eq!(CStr::from_ptr(d.name), name);
                    let mut prefix = [0 as c_char; PARAMETER_UI_PREFIX_SIZE];
                    assert_eq!(
                        factory().parameter_ui_prefix.unwrap()(
                            instance.base,
                            parameter as i32,
                            prefix.as_mut_ptr()
                        ),
                        3
                    );
                    assert_eq!(
                        CStr::from_ptr(prefix.as_ptr()).to_bytes(),
                        std::format!("V{}:", voice + 1).as_bytes()
                    );
                }
                let mut prefix = [0 as c_char; PARAMETER_UI_PREFIX_SIZE];
                assert_eq!(
                    factory().parameter_ui_prefix.unwrap()(
                        instance.base,
                        PITCH as i32,
                        prefix.as_mut_ptr()
                    ),
                    0
                );
                assert_eq!(CStr::from_ptr(prefix.as_ptr()), c"");
                assert_eq!(instance.values[output], if voice == 0 { 13 } else { 0 });
                assert_eq!(instance.values[cv], 0);
                assert_eq!(instance.values[mode], 0);
            }
            assert_eq!(page.num_params as usize, n as usize * 2 + 1);
            let rows = core::slice::from_raw_parts(page.params, page.num_params as usize);
            assert_eq!(rows[0] as usize, seam_parameter(n as usize));
            for segment in 0..n as usize {
                assert_eq!(
                    rows[segment * 2 + 1] as usize,
                    curve_parameter(n as usize, segment)
                );
                assert_eq!(
                    rows[segment * 2 + 2] as usize,
                    editor_parameter(n as usize, segment, 1).unwrap_or(seam_parameter(n as usize))
                );
            }
            let seam = *(*instance.base).parameters.add(seam_parameter(n as usize));
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
            for segment in 0..n as usize {
                assert_eq!(references[curve_parameter(n as usize, segment)], 1);
            }
            assert_eq!(references[seam_parameter(n as usize)], 2);
            assert_eq!(references[randomize_parameter(n as usize)], 1);
            let midi = midi_channel_parameter(n as usize);
            assert_eq!(references[midi], 1);
            assert_eq!(midi, GLOBALS + n as usize * 2 + 11 + (VOICE_COUNT - 1) * 3);
            let d = *(*instance.base).parameters.add(midi);
            assert_eq!((d.min, d.max, d.default, d.unit), (0, 17, 0, UNIT_ENUM));
            assert_eq!(CStr::from_ptr(*d.enum_strings), c"Off");
            assert_eq!(CStr::from_ptr(*d.enum_strings.add(1)), c"All");
            assert_eq!(CStr::from_ptr(*d.enum_strings.add(17)), c"16");
            let fold = fold_limit_parameter(n as usize);
            assert_eq!(fold, randomize_parameter(n as usize) + 10);
            assert_eq!(references[fold], 1);
            let d = *(*instance.base).parameters.add(fold);
            assert_eq!(CStr::from_ptr(d.name), c"Fold Limit");
            assert_eq!((d.min, d.max, d.default, d.unit), (0, 10, 2, UNIT_NONE));
            for (index, name) in shape_triggers(n as usize).into_iter().zip([
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
                (
                    centroid_parameter(n as usize),
                    c"Warp Position",
                    0,
                    255,
                    128,
                ),
                (strength_parameter(n as usize), c"Warp Amount", -100, 100, 0),
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
fn only_segment_count_is_specified_and_six_tables_are_always_allocated() {
    assert_eq!(Wavesmith::SPECIFICATIONS.len(), 1);
    assert_eq!(Wavesmith::SPECIFICATIONS[0].name, c"Segments");
    assert_eq!(unsafe { factory() }.num_specifications, 1);
    for segments in 2..=32 {
        let mut instance = Instance::new(segments);
        assert_eq!(
            Wavesmith::configuration(&[segments]).unwrap().voices,
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
            instance.change(centroid_parameter(n), 94);
            instance.change(strength_parameter(n), -68);
            instance.change(fold_limit_parameter(n), 7);
            let mut boundaries = std::vec![seam_parameter(n)];
            boundaries.extend((1..n).map(boundary_parameter));
            let curves: Vec<_> = (0..n).map(|segment| curve_parameter(n, segment)).collect();
            for (i, &parameter) in boundaries.iter().enumerate() {
                instance.change(parameter, (i as i16 * 3) - 40);
            }
            for (i, &parameter) in curves.iter().enumerate() {
                instance.change(parameter, 20 + i as i16);
            }
            let trigger = shape_triggers(n)[kind];
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
    let [all, boundaries, curves, zero, ..] = shape_triggers(8);
    for trigger in [all, boundaries, curves, zero] {
        instance.change(trigger, 1);
    }
    let boundary_values: Vec<_> = (1..8)
        .map(|i| instance.values[boundary_parameter(i)])
        .collect();
    instance.change(curve_parameter(8, 0), 73);
    let before = instance.values.clone();
    for trigger in [all, boundaries, curves, zero] {
        instance.change(trigger, 1);
    }
    assert_eq!(instance.values, before);
    instance.change(curves, 0);
    instance.change(curves, 1);
    assert!((0..8).any(|i| instance.values[curve_parameter(8, i)] != before[curve_parameter(8, i)]));
    assert_eq!(
        (1..8)
            .map(|i| instance.values[boundary_parameter(i)])
            .collect::<Vec<_>>(),
        boundary_values
    );
    instance.change(zero, 0);
    instance.change(zero, 1);
    assert!((0..8).all(|i| instance.values[curve_parameter(8, i)] == 0));
}

#[test]
fn waveform_boundary_triggers_preserve_curves_globals_and_independent_edges() {
    for n in 2..=32 {
        let mut instance = Instance::new(n);
        let n = n as usize;
        instance.change(SMOOTHING, 43);
        instance.change(centroid_parameter(n), 94);
        instance.change(strength_parameter(n), -68);
        for i in 0..n {
            instance.change(curve_parameter(n, i), 20 + i as i16);
        }
        let boundaries: Vec<_> = std::iter::once(seam_parameter(n))
            .chain((1..n).map(boundary_parameter))
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
            let trigger = shape_triggers(n)[4 + kind];
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
            instance.change(seam_parameter(n), 73);
            let held = instance.values.clone();
            for trigger in shape_triggers(n).into_iter().skip(4).take(kind + 1) {
                instance.change(trigger, 1);
            }
            assert_eq!(instance.values, held);
            instance.change(trigger, 0);
            instance.change(trigger, 1);
            assert_eq!(instance.values[seam_parameter(n)], -100);
        }
    }
}

#[test]
fn randomize_triggers_on_rising_edges_and_preserves_other_controls() {
    for n in 2..=32 {
        let mut instance = Instance::new(n);
        let n = n as usize;
        let trigger = randomize_parameter(n);
        instance.change(SMOOTHING, 43);
        instance.change(centroid_parameter(n), 37);
        instance.change(strength_parameter(n), -68);
        let before = instance.values.clone();
        instance.change(trigger, 1);
        let first = instance.values.clone();
        assert_eq!(first[centroid_parameter(n)], 37);
        assert_eq!(first[strength_parameter(n)], -68);
        assert_ne!(
            &first[GLOBALS..seam_parameter(n)],
            &before[GLOBALS..seam_parameter(n)]
        );
        assert_eq!(&first[..GLOBALS], &before[..GLOBALS]);
        assert!(first[GLOBALS..=seam_parameter(n)]
            .iter()
            .all(|v| (-100..=100).contains(v)));
        instance.change(trigger, 1);
        assert_eq!(instance.values, first);
        instance.change(trigger, 0);
        assert_eq!(&instance.values[..trigger], &first[..trigger]);
        instance.change(trigger, 1);
        assert_ne!(
            &instance.values[GLOBALS..seam_parameter(n)],
            &first[GLOBALS..seam_parameter(n)]
        );
        let wave = read_wave(n, |p| instance.values.get(p).copied());
        let seam = instance.values[seam_parameter(n)] as f32 * 0.01;
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
        let seam = seam_parameter(n);
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
                curve_parameter(n as usize, segment),
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
        assert_eq!(wave.segment_sample(0, 0.0), 0.0);
        assert_eq!(wave.segment_sample(n as usize - 1, 1.0), 0.0);
    }
}

#[test]
fn midi_gates_four_voices_ignores_cv_and_manual_pitch_and_restores_cv_when_off() {
    const FRAMES: usize = 24;
    let mut instance = Instance::new(8);
    for voice in 0..VOICE_COUNT {
        let [cv, output, mode] = voice_parameters(8, voice);
        instance.change(cv, voice as i16 + 1);
        instance.change(output, 13 + voice as i16);
        instance.change(mode, 1);
    }
    instance.change(PITCH, 0);
    instance.change(midi_channel_parameter(8), 2); // Channel 1.
    let mut samples = [0.0; LAST_BUS * FRAMES];
    let ticks = dsp::oscillator_tick_count();
    for _ in 0..60 {
        instance.render(&mut samples);
    }
    assert_eq!(dsp::oscillator_tick_count(), ticks);
    assert!(samples.iter().all(|&value| value == 0.0));
    instance.midi(0x91, 60, 100); // Wrong channel.
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count(), ticks);
    for note in [60, 64, 67, 71] {
        instance.midi(0x90, note, 100);
    }
    let mut crossings = [0usize; VOICE_COUNT];
    let mut last = [0.0; VOICE_COUNT];
    for block in 0..220 {
        // Deliberately unrelated CV; neither this nor manual Pitch affects MIDI.
        for voice in 0..VOICE_COUNT {
            samples[voice * FRAMES..(voice + 1) * FRAMES].fill(if block % 2 == 0 {
                8.0
            } else {
                -8.0
            });
        }
        if block == 110 {
            instance.change(PITCH, 127);
        }
        instance.render(&mut samples);
        if block >= 20 {
            for voice in 0..VOICE_COUNT {
                for &sample in &samples[(12 + voice) * FRAMES..(13 + voice) * FRAMES] {
                    if last[voice] < 0.0 && sample >= 0.0 {
                        crossings[voice] += 1;
                    }
                    last[voice] = sample;
                }
            }
        }
    }
    for (voice, frequency) in [261.6256f32, 329.6276, 391.9954, 493.8833]
        .into_iter()
        .enumerate()
    {
        assert!((crossings[voice] as f32 - frequency * 4800.0 / 48000.0).abs() < 2.0);
    }
    instance.midi(0x90, 74, 100); // Steals oldest voice (Voice 1).
    instance.midi(0x80, 60, 0); // Old key release must not release replacement.
    for _ in 0..30 {
        instance.render(&mut samples);
    }
    let ticks = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - ticks, FRAMES * VOICE_COUNT);
    for note in [74, 64, 67, 71] {
        instance.midi(0x90, note, 0);
    }
    for _ in 0..30 {
        instance.render(&mut samples);
    }
    let ticks = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count(), ticks);
    assert!(samples[12 * FRAMES..16 * FRAMES]
        .iter()
        .all(|&value| value == 0.0));
    instance.change(midi_channel_parameter(8), 0);
    instance.change(PITCH, 57);
    samples.fill(0.0);
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - ticks, FRAMES * VOICE_COUNT);
    assert!(samples[12 * FRAMES..16 * FRAMES]
        .iter()
        .any(|&value| value.abs() > 0.00001));
}

#[test]
fn midi_disabled_outputs_are_excluded_from_allocation_and_do_not_resume_old_notes() {
    const FRAMES: usize = 24;
    let mut instance = Instance::new(8);
    instance.change(OUTPUT_MODE, 1);
    instance.change(midi_channel_parameter(8), 1); // All channels.
    let mut samples = [0.0; LAST_BUS * FRAMES];
    for _ in 0..60 {
        instance.render(&mut samples);
    }
    for note in [60, 64, 67, 71, 74] {
        instance.midi(0x90, note, 100);
    }
    for _ in 0..30 {
        instance.render(&mut samples);
    }
    let ticks = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - ticks, FRAMES);
    instance.change(OUTPUT, 0);
    let ticks = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count(), ticks);
    instance.change(OUTPUT, 13);
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count(), ticks);
    assert!(samples[12 * FRAMES..13 * FRAMES]
        .iter()
        .all(|&value| value == 0.0));
    instance.midi(0x90, 67, 100);
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - ticks, FRAMES);
}

#[test]
fn four_voices_follow_independent_cv_pitches_and_none_output_skips_playback() {
    const FRAMES: usize = 24;
    let mut instance = Instance::new(32);
    let mut samples = [0.0; LAST_BUS * FRAMES];
    let ticks = dsp::oscillator_tick_count();
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - ticks, FRAMES);
    for voice in 0..VOICE_COUNT {
        let [cv, output, mode] = voice_parameters(32, voice);
        instance.change(cv, voice as i16 + 1);
        instance.change(output, 13 + voice as i16);
        instance.change(mode, 1);
    }
    let mut crossings = [0usize; VOICE_COUNT];
    let mut last = [0.0; VOICE_COUNT];
    for block in 0..160 {
        for voice in 0..VOICE_COUNT {
            samples[voice * FRAMES..(voice + 1) * FRAMES].fill(voice as f32);
        }
        let ticks = dsp::oscillator_tick_count();
        instance.render(&mut samples);
        assert_eq!(dsp::oscillator_tick_count() - ticks, VOICE_COUNT * FRAMES);
        if block >= 60 {
            for voice in 0..VOICE_COUNT {
                for &sample in &samples[(12 + voice) * FRAMES..(13 + voice) * FRAMES] {
                    if last[voice] < 0.0 && sample >= 0.0 {
                        crossings[voice] += 1;
                    }
                    last[voice] = sample;
                }
            }
        }
    }
    for (voice, &count) in crossings.iter().enumerate() {
        let expected = 220.0 * (1usize << voice) as f32 * 2400.0 / 48000.0;
        assert!(
            (count as f32 - expected).abs() < 2.0,
            "voice {voice}: {count} vs {expected}"
        );
    }
    for voice in 0..VOICE_COUNT {
        instance.change(voice_parameters(32, voice)[1], 0);
    }
    instance.change(curve_parameter(32, 0), 73);
    let ticks = dsp::oscillator_tick_count();
    for _ in 0..80 {
        samples.fill(2.5);
        instance.render(&mut samples);
        assert!(samples.iter().all(|&value| value == 2.5));
    }
    assert_eq!(dsp::oscillator_tick_count(), ticks);
    let [cv, output, mode] = voice_parameters(32, 2);
    instance.change(cv, 0);
    instance.change(output, 15);
    instance.change(mode, 1);
    samples.fill(0.0);
    instance.render(&mut samples);
    assert_eq!(dsp::oscillator_tick_count() - ticks, FRAMES);
    assert!(samples[14 * FRAMES..15 * FRAMES]
        .iter()
        .any(|&value| value.abs() > 0.001));
}

#[test]
fn voice_pitch_inputs_are_snapshotted_before_outputs_and_none_cv_uses_manual_pitch() {
    let mut instance = Instance::new(32);
    let [_, out1, mode1] = voice_parameters(32, 0);
    let [cv2, out2, mode2] = voice_parameters(32, 1);
    instance.change(out1, 1);
    instance.change(mode1, 1);
    instance.change(cv2, 1);
    instance.change(out2, 14);
    instance.change(mode2, 1);
    let mut samples = [0.0; LAST_BUS * 64];
    let mut last = [0.0; 2];
    let mut crossings = [0usize; 2];
    for block in 0..160 {
        // Bus 1 is both Voice 2's pitch input and Voice 1's destination.
        samples[..64].fill(1.0);
        instance.render(&mut samples);
        if block >= 60 {
            for (voice, bus) in [0, 13].into_iter().enumerate() {
                for &sample in &samples[bus * 64..(bus + 1) * 64] {
                    if last[voice] < 0.0 && sample >= 0.0 {
                        crossings[voice] += 1;
                    }
                    last[voice] = sample;
                }
            }
        }
    }
    for (voice, frequency) in [220.0f32, 440.0].into_iter().enumerate() {
        assert!((crossings[voice] as f32 - frequency * 6400.0 / 48000.0).abs() < 2.0);
    }
}

#[test]
fn voices_sharing_an_output_mix_after_a_single_replace_clear() {
    let mut reference: Option<Vec<f32>> = None;
    for enabled in [1, 2, 4] {
        let mut instance = Instance::new(8);
        for voice in 0..enabled {
            let [_, output, mode] = voice_parameters(8, voice);
            instance.change(output, 13);
            instance.change(mode, 1);
        }
        let mut samples = [0.0; LAST_BUS * 64];
        for _ in 0..80 {
            samples.fill(2.0);
            instance.render(&mut samples);
        }
        let output = samples[12 * 64..13 * 64].to_vec();
        if let Some(mono) = &reference {
            for (&actual, &expected) in output.iter().zip(mono) {
                assert!((actual - expected * enabled as f32).abs() < 0.00001);
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
    instance.change(PITCH_CV, 1);
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
    instance.change(curve_parameter(7, 2), 100);
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
    instance.change(curve_parameter(32, 0), 100);
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
fn centroid_marker_grid_and_handles_follow_redistributed_boundaries() {
    let mut instance = Instance::new(8);
    instance.event([0.5; 3], [2, 0]);
    instance.change(centroid_parameter(8), 94);
    for strength in [-100, 100] {
        instance.change(strength_parameter(8), strength);
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
    instance.change(LEVEL, 35);
    instance.change(centroid_parameter(8), 94);
    instance.change(strength_parameter(8), -68);
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
        for expected in ["L: Pitch", "C: Fine", "R: Level", "R click: Segment"] {
            assert!(text
                .iter()
                .any(|(_, y, value)| *y >= 57 && value == expected));
        }
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
            .any(|(_, _, value)| value == "R click: Warp"))
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
            instance.change(curve_parameter(n, segment), segment as i16 * 3 - 50);
        }
        instance.event([0.5; 3], [0, 0]);
        let before = instance.values.clone();
        let mut selected = 0i32;
        for delta in [-1i8, 1, 127, -128, 1, -1] {
            selected = (selected + delta as i32).rem_euclid(n as i32);
            instance.event([0.508; 3], [delta, 0]);
            assert_eq!(instance.values, before);
            instance.draw();
            let expected = std::format!("Curve {}", before[curve_parameter(n, selected as usize)]);
            TEXT.with(|text| {
                assert!(text
                    .borrow()
                    .iter()
                    .any(|(_, y, value)| *y == 10 && value == &expected))
            });
        }
        instance.event([0.508, 0.517, 0.508], [0, 0]);
        let edited = curve_parameter(n, selected as usize);
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
            instance.change(curve_parameter(n, selected), -37);
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
    instance.change(LEVEL, 30);
    instance.change(centroid_parameter(8), 25);
    instance.change(strength_parameter(8), 0);
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
        for expected in ["L: Warp Pos", "C: Warp Amt", "R: Smooth", "R click: Pitch"] {
            assert!(text
                .borrow()
                .iter()
                .any(|(_, y, s)| *y >= 57 && s == expected));
        }
    });
    for step in 1..=40 {
        instance.event([0.8 + step as f32 * 0.005; 3], [0, 0]);
    }
    assert_eq!(instance.values[SMOOTHING], 100);
    assert_eq!(instance.values[strength_parameter(8)], 100);
    let trigger = randomize_parameter(8);
    assert_eq!(instance.values[trigger], 0);
    assert_eq!(instance.values[centroid_parameter(8)], 255);
    assert_eq!(instance.values[LEVEL], 30);
    assert_eq!(instance.values[PITCH], 40);
    let first_shape = instance.values[GLOBALS..trigger].to_vec();
    assert_eq!(first_shape, before[GLOBALS..trigger]);
    instance.event([1.0; 3], [0, 0]);
    assert_eq!(&instance.values[GLOBALS..trigger], &first_shape);
    for step in (0..200).rev() {
        instance.event([step as f32 * 0.005; 3], [0, 0]);
    }
    assert_eq!(instance.values[SMOOTHING], 0);
    assert_eq!(instance.values[strength_parameter(8)], -100);
    assert_eq!(instance.values[trigger], 0);
    assert_eq!(instance.values[centroid_parameter(8)], 0);
    assert_eq!(&instance.values[GLOBALS..trigger], &first_shape);
    assert_eq!(instance.values[PITCH], 40);
    // Slow motion must accumulate instead of disappearing in integer rounding.
    for step in 1..=100 {
        instance.event([step as f32 * 0.0002; 3], [0, 0]);
    }
    assert!(instance.values[SMOOTHING] > 0);
    assert!(instance.values[strength_parameter(8)] > -100);
    assert!(instance.values[centroid_parameter(8)] > 0);
    assert_eq!(instance.values[PITCH], 40);
    instance.draw();
    TEXT.with(|text| {
        assert!(text.borrow().iter().any(|(x, y, s)| *x == 198
            && *y == 10
            && s == &std::format!("Smooth {}", instance.values[SMOOTHING])));
    });
    // Changing modes alongside pot movement re-primes pickup without edits.
    instance.change(strength_parameter(8), 0);
    // Visit Tuning, then click again to return to Segment mode.
    instance.event_buttons([0.5; 3], [0, 0], ENCODER_BUTTON_R, 0);
    instance.event_buttons([0.5; 3], [0, 0], 0, ENCODER_BUTTON_R);
    let before_switch = instance.values.clone();
    instance.event_buttons([0.5; 3], [0, 0], ENCODER_BUTTON_R, 0);
    assert_eq!(instance.values, before_switch);
    instance.event_buttons([0.5; 3], [0, 0], 0, ENCODER_BUTTON_R);
    instance.draw();
    TEXT.with(|text| assert!(text.borrow().iter().any(|(_, _, s)| s == "R click: Warp")));
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
    assert_eq!(editor_parameter(32, 0, 0), Some(seam_parameter(32)));
    assert!(instance.values[segment_parameter(0, 2)] > 0);
    assert_eq!(instance.values[SMOOTHING], 0);
    instance.event([0.51; 3], [31, 0]);
    let previous = instance.values[segment_parameter(31, 0)];
    instance.event([0.52; 3], [0, 0]);
    assert!(instance.values[segment_parameter(31, 0)] > previous);
    assert_eq!(editor_parameter(32, 31, 1), Some(seam_parameter(32)));
    let mut samples = [0.0; LAST_BUS * 64];
    for _ in 0..200 {
        instance.render(&mut samples);
    }
    instance.draw();
    TEXT.with(|text| {
        let text = text.borrow();
        for expected in [
            "L: Start",
            "C: Curve",
            "R: End",
            "Enc L: Segment",
            "R click: Warp",
        ] {
            assert!(text.iter().any(|(_, y, s)| *y >= 57 && s == expected));
        }
        assert_eq!(text.len(), 8);
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
unsafe extern "C" fn NT_algorithmIndex(_: *const NtAlgorithm) -> i32 {
    0
}
#[no_mangle]
unsafe extern "C" fn NT_parameterOffset() -> u32 {
    0
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
