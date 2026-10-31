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
        Self::with_tables(n, dsp::DEFAULT_TABLES as i32)
    }

    fn with_tables(n: i32, tables: i32) -> Self {
        unsafe {
            let specs = [n, tables];
            let f = factory();
            let mut req = NtAlgorithmRequirements {
                num_parameters: 0,
                sram: 0,
                dram: 0,
                dtc: 0,
                itc: 0,
            };
            f.calculate_requirements.unwrap()(&mut req, specs.as_ptr());
            assert_eq!(req.num_parameters as usize, GLOBALS + n as usize * 2 + 1);
            assert!(
                req.sram
                    < (dsp::blocks(tables as usize) * core::mem::size_of::<dsp::Block>() + 8000)
                        as u32
            );
            assert!(core::mem::size_of::<State>() < 1024);
            let alignment =
                distingnt_api::api::plugin::Registration::<Wavesmith, 2>::alignment(&specs)
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
            assert_eq!((*(*instance.base).parameter_pages).num_pages, 3);
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
            let page = &*pages.pages.add(2);
            assert_eq!(CStr::from_ptr(page.name), c"Waveform");
            assert_ne!(page.group, 0);
            assert_ne!(page.group, oscillator_group);
            assert_ne!(page.group, routing_group);
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
        }
    }
    for n in [0, 1, 33, -1] {
        assert!(Wavesmith::configuration(&[n, 6]).is_none());
    }
    assert_eq!(pluginEntry(SELECTOR_VERSION, 0), API_VERSION_14);
}

#[test]
fn table_specification_controls_real_allocations_for_every_supported_count() {
    let mut previous_size = 0;
    for tables in 1..=20 {
        for segments in [2, 32] {
            let mut instance = Instance::with_tables(segments, tables);
            assert_eq!(
                Wavesmith::configuration(&[segments, tables])
                    .unwrap()
                    .voices,
                dsp::blocks(tables as usize)
            );
            assert_eq!(unsafe { factory() }.num_specifications, 2);
            let mut samples = [0.0; LAST_BUS * 64];
            instance.change(PITCH, 105);
            for smoothing in [0, 100] {
                instance.change(SMOOTHING, smoothing);
                for _ in 0..40 {
                    instance.render(&mut samples);
                }
                assert!(samples.iter().all(|v| v.is_finite()));
                assert!(samples.iter().any(|v| v.abs() > 0.001));
                instance.draw();
            }
            if segments == 32 {
                if tables > 1 {
                    assert_eq!(
                        instance.layout.size() - previous_size,
                        core::mem::size_of::<dsp::Block>()
                    );
                }
                previous_size = instance.layout.size();
            }
        }
    }
    for tables in [-1, 0, 21] {
        assert!(Wavesmith::configuration(&[8, tables]).is_none());
        assert!(Wavesmith::create(&[8, tables], 48000).is_none());
    }
    assert!(Wavesmith::configuration(&[8]).is_none());
    assert_eq!(Wavesmith::SPECIFICATIONS[1].default, 6);
}

#[test]
fn randomize_triggers_on_rising_edges_and_preserves_other_controls() {
    for n in 2..=32 {
        let mut instance = Instance::new(n);
        let n = n as usize;
        let trigger = randomize_parameter(n);
        instance.change(SMOOTHING, 43);
        let before = instance.values.clone();
        instance.change(trigger, 1);
        let first = instance.values.clone();
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
        for _ in 0..32 {
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
                for _ in 0..32 {
                    instance.render(&mut samples);
                }
                instance.draw();
                TRACE_LINES.with(|lines| {
                    let lines = lines.borrow();
                    for trace in lines.chunks_exact(255) {
                        assert_eq!(trace.first().unwrap().1, trace.last().unwrap().3);
                    }
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
fn output_modes_routing_pitch_cv_and_live_shape_edits() {
    let mut instance = Instance::new(8);
    instance.change(OUTPUT, 64);
    instance.change(OUTPUT_MODE, 1);
    let mut samples = [0.0; LAST_BUS * 64];
    for _ in 0..40 {
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
    for _ in 0..40 {
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
fn dim_audio_trace_is_drawn_before_bright_editor_trace_with_or_without_smoothing() {
    let mut instance = Instance::new(7);
    instance.change(curve_parameter(7, 2), 100);
    let mut samples = [0.0; LAST_BUS * 64];
    let mut original = None;
    for smoothing in [0, 75, 0] {
        instance.change(SMOOTHING, smoothing);
        for _ in 0..40 {
            instance.render(&mut samples);
        }
        instance.draw();
        TRACE_LINES.with(|lines| {
            let lines = lines.borrow();
            assert_eq!(lines.len(), 510);
            assert!(lines[..255].iter().all(|line| line.4 == 4.0));
            assert!(lines[255..].iter().all(|line| line.4 == 15.0));
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
            // Solid grid lines use the lowest nonzero brightness.
            assert_eq!(colors.borrow().len(), 7);
            assert!(colors.borrow().iter().all(|&color| color == 1));
        });
        MARKER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[15, 15]));
    }
}

#[test]
fn high_pitch_changes_only_the_dim_audio_overlay() {
    let mut instance = Instance::new(32);
    instance.change(curve_parameter(32, 0), 100);
    let mut samples = [0.0; LAST_BUS * 64];
    for _ in 0..40 {
        instance.render(&mut samples);
    }
    instance.draw();
    let low = TRACE_LINES.with(|lines| lines.borrow().clone());
    instance.change(PITCH, 105);
    for _ in 0..40 {
        instance.render(&mut samples);
    }
    instance.draw();
    TRACE_LINES.with(|lines| {
        let lines = lines.borrow();
        assert_eq!(&lines[255..], &low[255..]);
        assert_ne!(&lines[..255], &low[..255]);
        assert!(lines[..255]
            .iter()
            .zip(&lines[255..])
            .any(|(audio, source)| (audio.1 - source.1).abs() > 0.1
                || (audio.3 - source.3).abs() > 0.1));
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
            for _ in 0..40 {
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
                    .any(|line| line.2 == 63.0 && (line.3 - 31.25).abs() < 0.001))
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
fn right_encoder_click_switches_pot_modes_without_jumps_and_reaches_all_ranges() {
    let mut instance = Instance::new(8);
    instance.event([0.8; 3], [2, 0]);
    instance.change(SMOOTHING, 25);
    instance.change(LEVEL, 30);
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
        assert!(!shapes.iter().any(|s| s.0 == SHAPE_RECTANGLE && s.2 == 14));
    });
    HEADER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[15, 15, 15]));
    TRACE_LINES.with(|lines| {
        let lines = lines.borrow();
        assert_eq!(lines.len(), 510);
        assert!(lines[..255].iter().all(|line| line.4 == 4.0));
        assert!(lines[255..].iter().all(|line| line.4 == 15.0));
    });
    GRID_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[1]));
    SHAPES.with(|shapes| {
        let shapes = shapes.borrow();
        assert!(shapes.contains(&(SHAPE_LINE, 0, 28, 255, 28)));
        assert!(!shapes
            .iter()
            .any(|s| s.0 == SHAPE_LINE && s.2 == 14 && s.4 == 42));
    });
    TEXT.with(|text| {
        for expected in ["L: Randomize", "C: Level", "R: Smooth", "R click: Segment"] {
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
    assert_eq!(instance.values[LEVEL], 100);
    let trigger = randomize_parameter(8);
    assert_eq!(instance.values[trigger], 1);
    assert_eq!(instance.values[PITCH], 40);
    let first_shape = instance.values[GLOBALS..trigger].to_vec();
    assert_ne!(first_shape, before[GLOBALS..trigger]);
    instance.event([1.0; 3], [0, 0]);
    assert_eq!(&instance.values[GLOBALS..trigger], &first_shape);
    for step in (0..200).rev() {
        instance.event([step as f32 * 0.005; 3], [0, 0]);
    }
    assert_eq!(instance.values[SMOOTHING], 0);
    assert_eq!(instance.values[LEVEL], 0);
    assert_eq!(instance.values[trigger], 0);
    assert_eq!(&instance.values[GLOBALS..trigger], &first_shape);
    assert_eq!(instance.values[PITCH], 40);
    // Slow motion must accumulate instead of disappearing in integer rounding.
    for step in 1..=100 {
        instance.event([step as f32 * 0.0002; 3], [0, 0]);
    }
    assert!(instance.values[SMOOTHING] > 0);
    assert!(instance.values[LEVEL] > 0);
    assert_eq!(instance.values[PITCH], 40);
    instance.draw();
    TEXT.with(|text| {
        assert!(text.borrow().iter().any(|(x, y, s)| *x == 198
            && *y == 10
            && s == &std::format!("Smooth {}", instance.values[SMOOTHING])));
    });
    // Changing modes alongside pot movement re-primes pickup without edits.
    let before_switch = instance.values.clone();
    instance.event_buttons([0.5; 3], [0, 0], ENCODER_BUTTON_R, 0);
    assert_eq!(instance.values, before_switch);
    instance.event_buttons([0.5; 3], [0, 0], 0, ENCODER_BUTTON_R);
    instance.draw();
    TEXT.with(|text| {
        assert!(text
            .borrow()
            .iter()
            .any(|(_, _, s)| s == "R click: Globals"))
    });
    SHAPES.with(|shapes| {
        let shapes = shapes.borrow();
        assert!(!shapes.contains(&(SHAPE_RECTANGLE, 0, 0, 255, 7)));
        assert!(!shapes.contains(&(SHAPE_RECTANGLE, 0, 0, 255, 12)));
        // Segment 3 remains selected after returning from Globals mode.
        assert!(shapes.contains(&(SHAPE_RECTANGLE, 63, 14, 95, 42)));
        assert_eq!(shapes.iter().filter(|s| s.0 == SHAPE_BOX).count(), 2);
    });
    HEADER_COLORS.with(|colors| assert_eq!(&*colors.borrow(), &[4, 4, 4]));
    TRACE_LINES.with(|lines| {
        let lines = lines.borrow();
        assert_eq!(lines.len(), 510);
        assert!(lines[..255].iter().all(|line| line.4 == 4.0));
        assert!(lines[255..].iter().all(|line| line.4 == 15.0));
    });
    GRID_COLORS.with(|colors| assert_eq!(colors.borrow().len(), 8));
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
            "R click: Globals",
        ] {
            assert!(text.iter().any(|(_, y, s)| *y >= 57 && s == expected));
        }
        assert_eq!(text.len(), 8);
        for expected in ["Randomize 0", "Level 50", "Smooth 0"] {
            assert!(text.iter().any(|(_, y, s)| *y == 10 && s == expected));
        }
        assert!(text
            .iter()
            .any(|(x, y, s)| *x == 106 && *y == 10 && s == "Level 50"));
        assert!(text
            .iter()
            .any(|(x, y, s)| *x == 198 && *y == 10 && s == "Smooth 0"));
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
