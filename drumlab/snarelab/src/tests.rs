//! Native host fixture for the registration, metadata, and render handoff.
extern crate std;

use super::*;
use core::{cell::Cell, ptr};
use distingnt_sys::{
    AudioWorker, NtAlgorithm, NtAlgorithmMemoryPtrs, NtAlgorithmRequirements, NtFactory, NtGlobals,
    NtStaticMemoryPtrs, NtStaticRequirements,
};
use std::{sync::OnceLock, vec, vec::Vec};

std::thread_local! {
    static ACTIVE: Cell<*mut NtAlgorithm> = const { Cell::new(ptr::null_mut()) };
    static WORK: Cell<Option<(AudioWorker, u32)>> = const { Cell::new(None) };
    static ACCEPT_WORK: Cell<bool> = const { Cell::new(true) };
    static PAGE_UPDATES: Cell<usize> = const { Cell::new(0) };
    static CYCLES: Cell<u32> = const { Cell::new(u32::MAX - 7) };
    static CYCLE_INCREMENT: Cell<u32> = const { Cell::new(13) };
    static AUDIO_WRITES: Cell<usize> = const { Cell::new(0) };
    static REENTER_RANDOMIZE: Cell<Option<usize>> = const { Cell::new(None) };
    static DEFER_MODEL_NOTIFICATION: Cell<bool> = const { Cell::new(false) };
    static DRAW_LABELS: std::cell::RefCell<Vec<std::string::String>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

#[no_mangle]
extern "C" fn NT_getCpuCycleCount() -> u32 {
    CYCLES.with(|counter| {
        let value = counter.get();
        counter.set(value.wrapping_add(CYCLE_INCREMENT.with(Cell::get)));
        value
    })
}

fn factory() -> &'static NtFactory {
    unsafe { &*(pluginEntry(SELECTOR_FACTORY_INFO, 0) as *const NtFactory) }
}

fn initialize_factory() {
    static DRAM: OnceLock<Vec<f32>> = OnceLock::new();
    DRAM.get_or_init(|| {
        let mut requirements = NtStaticRequirements { dram: 0 };
        unsafe { factory().calculate_static_requirements.unwrap()(&mut requirements) };
        assert_eq!(requirements.dram, lookup::BYTES);
        let mut dram = vec![f32::NAN; requirements.dram as usize / 4];
        let mut memory = NtStaticMemoryPtrs {
            dram: dram.as_mut_ptr().cast(),
        };
        unsafe { factory().initialise.unwrap()(&mut memory, &requirements) };
        assert!(dram.iter().all(|sample| sample.is_finite()));
        assert_eq!(dram.first(), Some(&0.0));
        assert_eq!(dram.last(), Some(&2.0));
        dram
    });
}

#[no_mangle]
extern "C" fn rust_nt_globals() -> *const NtGlobals {
    static GLOBALS: NtGlobals = NtGlobals {
        sample_rate: 48_000,
        max_frames_per_step: 128,
        work_buffer: ptr::null_mut(),
        work_buffer_size_bytes: 0,
        stream_size_bytes: 0,
        stream_buffer_size_bytes: 0,
    };
    &GLOBALS
}

#[no_mangle]
extern "C" fn NT_algorithmIndex(_: *const NtAlgorithm) -> i32 {
    3
}

#[no_mangle]
extern "C" fn NT_parameterOffset() -> u32 {
    7
}

#[no_mangle]
unsafe extern "C" fn NT_intToString(buffer: *mut c_char, value: i32) -> i32 {
    use std::string::ToString;
    let text = value.to_string();
    ptr::copy_nonoverlapping(text.as_ptr(), buffer.cast(), text.len());
    buffer.add(text.len()).write(0);
    text.len() as i32
}

#[no_mangle]
unsafe extern "C" fn NT_floatToString(buffer: *mut c_char, value: f32, decimals: i32) -> i32 {
    let text = std::format!("{value:.precision$}", precision = decimals as usize);
    ptr::copy_nonoverlapping(text.as_ptr(), buffer.cast(), text.len());
    buffer.add(text.len()).write(0);
    text.len() as i32
}

#[no_mangle]
unsafe extern "C" fn NT_drawText(_: i32, _: i32, text: *const c_char, _: i32, _: u8, _: u8) {
    let text = CStr::from_ptr(text).to_str().unwrap();
    DRAW_LABELS.with_borrow_mut(|labels| {
        labels.push(text.into());
    });
}

#[no_mangle]
extern "C" fn NT_drawShapeI(_: u8, _: i32, _: i32, _: i32, _: i32, _: i32) {}

#[no_mangle]
extern "C" fn NT_setParameterFromAudio(algorithm: u32, parameter: u32, value: i16) {
    assert_eq!(algorithm, 3);
    AUDIO_WRITES.with(|writes| writes.set(writes.get() + 1));
    let base = ACTIVE.with(Cell::get);
    let local = (parameter - NT_parameterOffset()) as usize;
    let is_model = matches!(parameter_layer(local as i32, MAX_LAYERS), Some((_, 0)));
    unsafe {
        if is_model {
            assert_eq!((*base).v.add(RANDOMIZE_LAYERS).read(), 1);
        }
        if let Some(action) = REENTER_RANDOMIZE.with(Cell::take) {
            assert_eq!((*base).v.add(action).read(), 1);
            factory().parameter_changed.unwrap()(base, action as i32);
        }
        let def = *(*base).parameters.add(local);
        (*base)
            .v
            .add(local)
            .cast_mut()
            .write(value.clamp(def.min, def.max));
        if !is_model || !DEFER_MODEL_NOTIFICATION.with(Cell::get) {
            factory().parameter_changed.unwrap()(base, local as i32);
        }
    }
}

#[no_mangle]
extern "C" fn NT_updateParameterDefinition(algorithm: u32, local_parameter: u32) {
    assert_eq!(algorithm, 3);
    assert!(matches!(
        parameter_layer(local_parameter as i32, MAX_LAYERS),
        Some((_, LAYER_FIRST_CONTROL..))
    ));
    // Simulate a host that clamps the value on a definition change.
    let base = ACTIVE.with(Cell::get);
    unsafe {
        let current = (*base).v.add(local_parameter as usize).read();
        NT_setParameterFromAudio(algorithm, local_parameter + NT_parameterOffset(), current);
    }
}

#[no_mangle]
extern "C" fn NT_updateParameterPages(algorithm: u32) {
    assert_eq!(algorithm, 3);
    PAGE_UPDATES.with(|updates| updates.set(updates.get() + 1));
}

#[no_mangle]
extern "C" fn NT_audioWorkerSubmit(
    callback: Option<AudioWorker>,
    algorithm: u32,
    data: u32,
) -> bool {
    assert_eq!(algorithm, 3);
    if !ACCEPT_WORK.with(Cell::get) {
        return false;
    }
    WORK.with(|pending| {
        assert!(pending.get().is_none());
        pending.set(Some((callback.unwrap(), data)));
    });
    true
}

struct Host {
    _sram: Vec<u64>,
    dram: Vec<f32>,
    specs: [i32; 2],
    values: Vec<i16>,
    base: *mut NtAlgorithm,
}

impl Host {
    fn new(layers: i32) -> Self {
        let host = Self::uninitialized(layers);
        host.notify(TRIGGER);
        host
    }

    fn with_basic_mix() -> Self {
        let mut host = Self::new(3);
        host.set(model_index(0), Model::Modal as i16);
        host.set(model_index(1), Model::Wires as i16);
        host.set(model_index(2), Model::Off as i16);
        host.set(model_index(1) + LAYER_LEVEL, 75);
        host.set(GAIN, -70);
        host
    }

    fn uninitialized(layers: i32) -> Self {
        Self::with_specs([layers, DEFAULT_OUTPUT_BUFFERS as i32])
    }

    fn with_specs(specs: [i32; 2]) -> Self {
        initialize_factory();
        let [layers, buffers] = specs;
        let mut req = NtAlgorithmRequirements {
            num_parameters: 0,
            sram: 0,
            dram: 0,
            dtc: 0,
            itc: 0,
        };
        unsafe { factory().calculate_requirements.unwrap()(&mut req, specs.as_ptr()) };
        assert_eq!(
            req.num_parameters as usize,
            extra_control_index(layers as usize)
        );
        assert_eq!(
            req.dram as usize,
            (((buffers + layers) as usize) * render::max_frames(48_000)) * 4
                + 2 * MIX_TILE_FRAMES * 4
        );
        let mut sram = vec![0; (req.sram as usize).div_ceil(8)];
        let mut dram = vec![f32::NAN; req.dram as usize / 4];
        let mem = NtAlgorithmMemoryPtrs {
            sram: sram.as_mut_ptr().cast(),
            dram: dram.as_mut_ptr().cast(),
            dtc: ptr::null_mut(),
            itc: ptr::null_mut(),
        };
        let base = unsafe { factory().construct.unwrap()(&mem, &req, specs.as_ptr()) };
        assert!(!base.is_null());
        assert!(dram.iter().all(|&sample| sample == 0.0));
        let definitions =
            unsafe { core::slice::from_raw_parts((*base).parameters, req.num_parameters as usize) };
        let mut values: Vec<i16> = definitions.iter().map(|def| def.default).collect();
        unsafe { (*base).v = values.as_mut_ptr() };
        ACTIVE.with(|active| active.set(base));
        PAGE_UPDATES.with(|updates| updates.set(0));
        Self {
            _sram: sram,
            dram,
            specs,
            values,
            base,
        }
    }

    fn set(&mut self, parameter: usize, value: i16) {
        self.values[parameter] = value;
        self.notify(parameter);
    }

    fn notify(&self, parameter: usize) {
        unsafe { factory().parameter_changed.unwrap()(self.base, parameter as i32) };
    }

    fn page(&self, page: usize) -> (u8, u8, Vec<u8>) {
        unsafe {
            let page = *(*(*self.base).parameter_pages).pages.add(page);
            (
                page.group,
                page.num_params,
                core::slice::from_raw_parts(page.params, page.num_params as usize).to_vec(),
            )
        }
    }

    fn page_name(&self, page: usize) -> &CStr {
        unsafe { CStr::from_ptr((*(*(*self.base).parameter_pages).pages.add(page)).name) }
    }

    fn step(&mut self, trigger: [f32; 4]) -> [f32; 4] {
        let writes = AUDIO_WRITES.with(Cell::get);
        let page_updates = PAGE_UPDATES.with(Cell::get);
        let mut buses = [0.0; 4 * LAST_BUS];
        let input = (self.values[TRIGGER] as usize - 1) * 4;
        let output = (self.values[OUTPUT] as usize - 1) * 4;
        buses[input..input + 4].copy_from_slice(&trigger);
        unsafe { factory().step.unwrap()(self.base, buses.as_mut_ptr(), 1) };
        assert_eq!(AUDIO_WRITES.with(Cell::get), writes);
        assert_eq!(PAGE_UPDATES.with(Cell::get), page_updates);
        core::array::from_fn(|frame| buses[output + frame])
    }

    fn run_slice(&mut self) -> usize {
        let (callback, slot) = WORK.with(Cell::take).expect("queued render");
        unsafe { callback(self.base, slot) };
        slot as usize
    }

    fn finish(&mut self) -> (usize, usize) {
        let slot = self.run_slice();
        for slices in 1..256 {
            self.step([0.0; 4]);
            if WORK.with(Cell::get).is_none() {
                return (slot, slices);
            }
            assert_eq!(self.run_slice(), slot);
        }
        panic!("render did not finish");
    }

    fn sample(&self, slot: usize, frame: usize) -> f32 {
        let start: usize = (0..slot)
            .map(|index| SnareLab::dram_buffer_len_at(&self.specs, 48_000, index))
            .sum();
        self.dram[start + frame]
    }

    fn layer_slot(&self, layer: usize) -> usize {
        layer_slot(output_buffer_count(&self.specs).unwrap(), layer)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if WORK.with(Cell::get).is_some() {
            self.run_slice();
        }
        ACTIVE.with(|active| active.set(ptr::null_mut()));
    }
}

#[test]
fn model_picker_groups_bodies_then_wires_then_rim_shot() {
    let ordered = [
        (Model::Off, c"Off"),
        (Model::PitchSweep, c"Pitch Sweep"),
        (Model::Resonator, c"Resonator"),
        (Model::Modal, c"Modal Body"),
        (Model::TwoStage, c"Two-Stage"),
        (Model::Wires, c"Snare Wires"),
        (Model::ResonantWires, c"Reson. Wires"),
        (Model::SweepWires, c"Sweep Wires"),
        (Model::Rim, c"Rim Shot"),
    ];
    for (value, (model, label)) in ordered.into_iter().enumerate() {
        assert_eq!(model as usize, value);
        assert!(Model::from_value(value as i16) == model);
        assert_eq!(MODEL_NAMES[value], label);
    }
}

#[test]
fn metadata_keeps_layer_ordinals_across_specifications_and_restores_models() {
    let mut reference = Vec::new();
    for layers in MIN_LAYERS..=MAX_LAYERS {
        for buffers in [
            OUTPUT_BUFFERS_SPECIFICATION.min as usize,
            MAX_OUTPUT_BUFFERS,
        ] {
            let mut host = Host::with_specs([layers as i32, buffers as i32]);
            let second = model_index(1);
            assert_eq!(&host.values[..GLOBALS], &[2, 14, 1, 0, -1, 0]);
            assert_eq!(host.page_name(1), c"1-Two-Stage");
            assert_eq!(host.page_name(2), c"2-Reson. Wires");
            let expected: &[(Model, &[i16])] = &[
                (Model::TwoStage, &[174, 485, 107, 167, 100]),
                (Model::ResonantWires, &[186, 311, 100, 100, 1674, 100, 0]),
                (Model::Rim, &[979, 16, 5900, 37, 0, 100, 100, 100, 100, 100]),
                (Model::Off, &[]),
                (Model::Off, &[]),
            ];
            for (layer, &(model, controls)) in expected.iter().take(layers).enumerate() {
                let base = model_index(layer);
                assert_eq!(
                    &host.values[base..base + LAYER_FIRST_CONTROL],
                    &[model as i16, 0, 100, 0, 0]
                );
                for (slot, &value) in controls.iter().enumerate() {
                    assert_eq!(host.values[control_index(layer, slot)], value);
                }
            }
            if layers >= 3 {
                assert_eq!(host.page_name(3), c"3-Rim Shot");
            }
            host.values[second] = Model::Rim as i16;
            host.values[second + LAYER_FIRST_CONTROL] = 2345;
            host.values[RANDOMIZE_LAYERS] = 1;
            host.notify(TRIGGER);
            assert_eq!(host.values[RANDOMIZE_LAYERS], 0);
            assert_eq!(host.values[second + LAYER_FIRST_CONTROL], 2345);
            assert_eq!(host.page_name(2), c"2-Rim Shot");
            let pages: Vec<_> = (0..=layers).map(|page| host.page(page)).collect();
            assert_eq!(pages[..reference.len()], reference);
            reference = pages;
        }
    }
}

#[test]
fn feeling_lucky_replaces_the_stack_and_reset_survives_reentrant_host_notifications() {
    for layers in [2, 3, 5] {
        let mut host = Host::new(layers);
        let mut outcomes = [false; 2];
        for run in 0..48 {
            if layers == 5 {
                host.set(model_index(4) + LAYER_SOLO, 1);
                host.set(model_index(4) + LAYER_MUTE, 1);
            }
            REENTER_RANDOMIZE.with(|action| action.set(Some(RANDOMIZE_LAYERS)));
            DEFER_MODEL_NOTIFICATION.with(|defer| defer.set(run % 2 == 0));
            host.set(RANDOMIZE_LAYERS, 1);
            DEFER_MODEL_NOTIFICATION.with(|defer| defer.set(false));
            assert_eq!(host.values[RANDOMIZE_LAYERS], 0);
            for layer in 0..layers as usize {
                let base = model_index(layer);
                let model = Model::from_value(host.values[base]);
                let level = host.values[base + LAYER_LEVEL];
                assert_eq!(host.values[base + LAYER_SOLO], 0);
                assert_eq!(host.values[base + LAYER_MUTE], 0);
                match layer {
                    0 => {
                        assert!(matches!(
                            model,
                            Model::PitchSweep | Model::Resonator | Model::Modal | Model::TwoStage
                        ));
                        assert_eq!(level, 100);
                    }
                    1 => {
                        assert!(matches!(
                            model,
                            Model::Wires | Model::SweepWires | Model::ResonantWires
                        ));
                        assert!((55..=85).contains(&level));
                    }
                    2 => {
                        assert!(matches!(model, Model::Off | Model::Rim));
                        outcomes[usize::from(model == Model::Rim)] = true;
                        if model == Model::Rim {
                            assert!((15..=45).contains(&level));
                        }
                    }
                    _ => assert!(model == Model::Off),
                }
                for (offset, control) in model.controls().iter().enumerate() {
                    let raw = host.values[control_index(layer, offset)];
                    let [min, max] = control.range;
                    assert_eq!((raw - min) % control.step, 0);
                    assert!((min..=max).contains(&raw));
                    assert!(
                        f32::from(raw)
                            >= 0.6 * f32::from(control.initial) + 0.4 * f32::from(min)
                                - 0.5 * f32::from(control.step)
                    );
                    assert!(
                        f32::from(raw)
                            <= 0.6 * f32::from(control.initial)
                                + 0.4 * f32::from(max)
                                + 0.5 * f32::from(control.step)
                    );
                }
            }
        }
        if layers > 2 {
            assert_eq!(outcomes, [true, true]);
        }
        let level = host.values[model_index(1) + LAYER_LEVEL];
        let model = host.values[model_index(1)];
        let action = model_index(1) + LAYER_RANDOMIZE;
        host.set(action, 1);
        assert_eq!(host.values[action], 0);
        assert_eq!(host.values[model_index(1)], model);
        assert_eq!(host.values[model_index(1) + LAYER_LEVEL], level);
    }
}

#[test]
fn render_yields_resumes_keeps_unchanged_layer_cache_and_tapers_only_capped_mix() {
    let mut host = Host::with_basic_mix();
    let wire = model_index(1);
    let body_cache = host.layer_slot(0);
    let wire_cache = host.layer_slot(1);
    host.step([0.0; 4]);
    host.run_slice();
    ACCEPT_WORK.with(|accept| accept.set(false));
    host.step([0.0; 4]);
    assert!(WORK.with(Cell::get).is_none());
    assert_eq!(host.step([5.0, 0.0, 0.0, 0.0]), [0.0; 4]);
    ACCEPT_WORK.with(|accept| accept.set(true));
    host.step([0.0; 4]);
    let (short, slices) = host.finish();
    assert!(slices > 1);
    assert_eq!(host.sample(short, 19_103), 0.0);
    assert_eq!(host.sample(short, 19_104), 0.0);
    assert!(host.sample(short, 17_000).abs() > 0.000001);
    let body: Vec<_> = (0..8192).map(|i| host.sample(body_cache, i)).collect();
    host.set(wire + LAYER_FIRST_CONTROL + 1, 500);
    host.step([0.0; 4]);
    let (long, _) = host.finish();
    assert_ne!(short, long);
    assert_eq!(host.sample(long, 35_999), 0.0);
    assert_ne!(host.sample(wire_cache, 35_999), 0.0);
    assert!(host.sample(long, 35_950).abs() < host.sample(wire_cache, 35_950).abs() * 0.01);
    assert!((0..8192).all(|i| host.sample(body_cache, i) == body[i]));
    unsafe { factory().draw.unwrap()(host.base) };
    assert!(DRAW_LABELS.with_borrow(|labels| labels.iter().any(|label| label == "750 ms")));
}

#[test]
fn three_layer_mix_uses_cached_samples_and_master_processing() {
    let mut host = Host::with_basic_mix();
    let rim = model_index(2);
    host.set(rim, Model::Rim as i16);
    host.set(control_index(2, 5), 0);
    host.set(rim + LAYER_LEVEL, 30);
    host.set(MASTER_DRIVE, 120);
    host.set(GAIN, -70);
    host.step([0.0; 4]);
    let (slot, _) = host.finish();
    let caches = [host.layer_slot(0), host.layer_slot(1), host.layer_slot(2)];
    let mut dry = vec![0.0_f32; 19_104];
    for (frame, sample) in dry.iter_mut().enumerate() {
        *sample = host.sample(caches[0], frame)
            + 0.75 * host.sample(caches[1], frame)
            + 0.3 * host.sample(caches[2], frame);
    }
    let peak = dry
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max);
    let ceiling = peak.max(1.0);
    let boost = 10.0_f32.powf(12.0 / 20.0);
    let gain = 10.0_f32.powf(-7.0 / 20.0);
    let trim_gain = gain / ((peak * boost).min(ceiling) * gain).max(1.0);
    for frame in [1, 145, 632, 2023, 3000, 8800, 12000, 19080] {
        let expected = (dry[frame] * boost).clamp(-ceiling, ceiling) * trim_gain;
        assert!((host.sample(slot, frame) - expected).abs() < 0.00001);
    }
}

#[test]
fn selecting_a_captured_model_on_another_layer_loads_its_patch() {
    let mut host = Host::new(2);
    let second = model_index(1);
    for (model, expected) in [
        (
            Model::Rim,
            &[979, 16, 5900, 37, 0, 100, 100, 100, 100, 100][..],
        ),
        (Model::TwoStage, &[174, 485, 107, 167, 100][..]),
        (
            Model::ResonantWires,
            &[186, 311, 100, 100, 1674, 100, 0][..],
        ),
    ] {
        host.set(second, model as i16);
        for (slot, value) in expected.iter().enumerate() {
            assert_eq!(host.values[control_index(1, slot)], *value);
        }
    }
}

#[test]
fn punch_layers_are_summed_before_shaping_and_clean_wires_are_added_after() {
    let mut host = Host::with_basic_mix();
    host.set(model_index(0), Model::TwoStage as i16);
    host.set(model_index(2), Model::Rim as i16);
    let rim_level = model_index(2) + LAYER_LEVEL;
    host.set(rim_level, 50);
    host.set(control_index(2, 5), 60);
    host.set(control_index(2, 9), 115);
    let scope = host.page(3).2;
    assert_eq!(scope.last(), Some(&(extra_control_index(2) as u8)));
    assert_eq!(host.values[control_index(2, 9)], 115);
    host.step([0.0; 4]);
    let (output, _) = host.finish();
    let body_cache = host.layer_slot(0);
    let wires_cache = host.layer_slot(1);
    let rim_cache = host.layer_slot(2);
    let master = 10.0_f32.powf(-7.0 / 20.0);
    for frame in [217, 501, 731, 954, 3100, 4300] {
        let ms = frame as f32 / 48.0;
        let p = ((ms - 4.0) / 10.0).clamp(0.0, 1.0);
        let scaled = (host.sample(body_cache, frame) + 0.3 * host.sample(rim_cache, frame))
            * (1.0 + 0.0961148 * (1.0 - p * p * (3.0 - 2.0 * p)));
        let punch = scaled / (1.0 + 0.24969612 * scaled.powi(4)).powf(0.25);
        let clean = 0.75 * host.sample(wires_cache, frame) + 0.2 * host.sample(rim_cache, frame);
        assert!((host.sample(output, frame) - (clean + punch) * master).abs() < 0.00001);
    }
}
