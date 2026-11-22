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
    #[cfg(feature = "profiling")]
    static PROFILE_PACKETS: std::cell::RefCell<Vec<Vec<u8>>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static CYCLES: Cell<u32> = const { Cell::new(u32::MAX - 7) };
    static AUDIO_WRITES: Cell<usize> = const { Cell::new(0) };
    static REENTER_RANDOMIZE: Cell<Option<usize>> = const { Cell::new(None) };
    static DEFER_MODEL_NOTIFICATION: Cell<bool> = const { Cell::new(false) };
    static DRAW_LABELS: std::cell::RefCell<Vec<std::string::String>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static WAVE_SEGMENTS: Cell<usize> = const { Cell::new(0) };
    static WAVE_ENDS: Cell<usize> = const { Cell::new(0) };
}

#[no_mangle]
extern "C" fn NT_getCpuCycleCount() -> u32 {
    CYCLES.with(|counter| {
        let value = counter.get();
        counter.set(value.wrapping_add(13));
        value
    })
}

#[cfg(feature = "profiling")]
#[no_mangle]
unsafe extern "C" fn NT_sendMidiSysEx(destination: u32, data: *const u8, count: u32, end: bool) {
    assert_eq!(destination, DESTINATION_USB);
    assert!(end);
    let message = core::slice::from_raw_parts(data, count as usize);
    PROFILE_PACKETS.with(|packets| packets.borrow_mut().push(message.to_vec()));
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
extern "C" fn NT_drawShapeI(kind: u8, x0: i32, y0: i32, x1: i32, y1: i32, colour: i32) {
    assert!((0..=255).contains(&x0) && (0..=255).contains(&x1));
    assert!((12..=63).contains(&y0) && (12..=63).contains(&y1));
    if colour == 1 {
        assert_eq!(kind, SHAPE_LINE);
        assert_eq!((x0, x1), (1, 254));
        assert!(matches!(y0, 12 | 50) && y0 == y1);
    }
    if colour == 8 {
        assert_eq!(kind, if x0 == x1 { SHAPE_POINT } else { SHAPE_LINE });
        assert_eq!(y0, y1);
        WAVE_ENDS.with(|count| count.set(count.get() + 1));
    } else if colour == 15 {
        let shape = if x0 != x1 {
            SHAPE_RECTANGLE
        } else if y0 == y1 {
            SHAPE_POINT
        } else {
            SHAPE_LINE
        };
        assert_eq!(kind, shape);
        assert!((1..=254).contains(&x0));
        assert!((12..=50).contains(&y0) && (y0..=50).contains(&y1));
        if y0 != y1 {
            WAVE_SEGMENTS.with(|count| count.set(count.get() + 1));
        }
    }
}

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
        assert_eq!(req.num_parameters as usize, model_index(layers as usize));
        assert_eq!(
            req.dram as usize,
            (((buffers + layers) as usize) * render::max_frames(48_000)
                + layers as usize * render::layer_cache_frames(48_000))
                * 4
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

    fn step(&mut self, trigger: [f32; 4]) -> [f32; 4] {
        let writes = AUDIO_WRITES.with(Cell::get);
        let page_updates = PAGE_UPDATES.with(Cell::get);
        let mut buses = [0.0; 4 * LAST_BUS];
        buses[..4].copy_from_slice(&trigger);
        unsafe { factory().step.unwrap()(self.base, buses.as_mut_ptr(), 1) };
        assert_eq!(AUDIO_WRITES.with(Cell::get), writes);
        assert_eq!(PAGE_UPDATES.with(Cell::get), page_updates);
        core::array::from_fn(|frame| buses[4 * (13 - 1) + frame])
    }

    fn complete(&mut self) -> usize {
        let (callback, slot) = WORK.with(Cell::take).expect("queued render");
        unsafe { callback(self.base, slot) };
        slot as usize
    }

    fn wave(&self, slot: usize, frame: usize) -> f32 {
        self.sample(slot, frame) * 5.0
    }

    fn sample(&self, slot: usize, frame: usize) -> f32 {
        let start: usize = (0..slot)
            .map(|index| KickLab::dram_buffer_len_at(&self.specs, 48_000, index))
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
            self.complete();
        }
        ACTIVE.with(|active| active.set(ptr::null_mut()));
    }
}

#[test]
fn output_buffer_specification_sets_pool_size_without_changing_parameter_pages() {
    let last_layer = MAX_LAYERS - 1;
    assert_eq!(
        KickLab::SPECIFICATIONS[1].default,
        DEFAULT_OUTPUT_BUFFERS as i32
    );
    assert!(KickLab::configuration(&[4, 1]).is_none());
    assert!(KickLab::configuration(&[4, 11]).is_none());

    let mut reference_pages = None;
    for count in [
        MIN_OUTPUT_BUFFERS,
        DEFAULT_OUTPUT_BUFFERS,
        MAX_OUTPUT_BUFFERS,
    ] {
        let mut host = Host::with_specs([MAX_LAYERS as i32, count as i32]);
        host.notify(TRIGGER);
        host.set(model_index(last_layer), 1);
        let pages: Vec<_> = (0..=MAX_LAYERS).map(|page| host.page(page)).collect();
        if let Some(reference) = &reference_pages {
            assert_eq!(&pages, reference);
        } else {
            reference_pages = Some(pages);
        }
        host.step([0.0; 4]);
        assert_eq!(host.complete(), 0);
        let cache = host.layer_slot(last_layer);
        assert!((1..128).any(|frame| host.sample(cache, frame).abs() > 0.001));
    }
}

#[cfg(not(feature = "profiling-hold-playback"))]
#[test]
fn two_output_buffers_wait_for_a_playing_tail_before_reusing_its_slot() {
    let mut host = Host::with_specs([1, 2]);
    host.notify(TRIGGER);
    let level = model_index(0) + LAYER_LEVEL;
    host.set(model_index(0) + LAYER_FIRST_CONTROL + 1, 45);
    host.step([0.0; 4]);
    assert_eq!(host.complete(), 0);
    host.step([5.0, 0.0, 0.0, 0.0]);

    host.set(level, 70);
    host.step([0.0; 4]);
    assert_eq!(host.complete(), 1);
    host.step([5.0, 0.0, 0.0, 0.0]);

    host.set(level, 60);
    host.step([0.0; 4]);
    assert!(WORK.with(Cell::get).is_none());
    for _ in 0..1000 {
        host.step([0.0; 4]);
        if WORK.with(Cell::get).is_some() {
            break;
        }
    }
    assert_eq!(host.complete(), 0);
}

#[test]
fn a_new_instance_starts_with_the_captured_pitch_sweep_patch_on_layer_one() {
    let mut host = Host::new(2);
    let first = model_index(0);
    let second = model_index(1);
    let defaults = [900, 500, 164, 10, 50, 15, 20, 3000, 160];
    assert_eq!(
        &host.values[first..first + LAYER_FIRST_CONTROL],
        &[1, 0, 100, 0, 0]
    );
    let first_controls = first + LAYER_FIRST_CONTROL;
    assert_eq!(
        &host.values[first_controls..first_controls + defaults.len()],
        &defaults
    );
    assert_eq!(host.values[second], 0);

    host.set(first_controls, 450);
    host.set(first, 2);
    for base in [first, second] {
        host.set(base, 1);
        let controls = base + LAYER_FIRST_CONTROL;
        assert_eq!(&host.values[controls..controls + defaults.len()], &defaults);
    }
}

#[test]
fn model_controls_refresh_on_notification_and_preserve_restored_values() {
    let mut host = Host::uninitialized(2);
    let first = model_index(0);
    let second = model_index(1);
    assert_eq!(
        unsafe { (*(*host.base).parameters.add(first)).unit },
        UNIT_CONFIRM
    );
    let mut name = [0; PARAMETER_STRING_SIZE];
    let length = unsafe {
        factory().parameter_string.unwrap()(host.base, first as i32, 3, name.as_mut_ptr())
    };
    assert_eq!(length, 9);
    assert_eq!(unsafe { CStr::from_ptr(name.as_ptr()) }, c"Bit Logic");
    assert_eq!(host.values[first], 1);
    assert!(WORK.with(Cell::get).is_none());
    host.step([0.0; 4]);
    assert!(WORK.with(Cell::get).is_none());
    host.values[first] = 3;
    host.values[first + LAYER_FIRST_CONTROL + 5] = 415; // Outside the Pitch Sweep slot's range.
    host.values[first + LAYER_FIRST_CONTROL + 8] = 2; // OR
    host.values[first + LAYER_LEVEL] = 72;
    host.values[first + LAYER_SOLO] = 1;
    host.values[first + LAYER_MUTE] = 1;
    host.notify(first);
    assert_eq!(host.values[first + LAYER_FIRST_CONTROL + 5], 415);
    assert_eq!(host.values[first + LAYER_FIRST_CONTROL + 8], 2);
    assert_eq!(
        host.page(1),
        (1, 15, (first as u8..(first + LAYER_FIELDS) as u8).collect())
    );
    assert_eq!(host.page(2), (1, 1, vec![second as u8]));
    assert_eq!(PAGE_UPDATES.with(Cell::get), 1);

    host.set(first, 2);
    assert_eq!(host.values[first], 2);
    assert_eq!(host.values[first + LAYER_LEVEL], 72);
    assert_eq!(
        &host.values[first + LAYER_SOLO..=first + LAYER_MUTE],
        &[1, 1]
    );
    let controls = first + LAYER_FIRST_CONTROL;
    assert_eq!(
        &host.values[controls..controls + MODAL_DRUM.len()],
        &[1100, 400, 150, 60, 20, 25, 50, 25]
    );
    assert_eq!(host.page(1).1, 13);
    assert_eq!(
        &host.page(1).2[..5],
        &[
            first as u8,
            (first + LAYER_RANDOMIZE) as u8,
            (first + LAYER_LEVEL) as u8,
            (first + LAYER_SOLO) as u8,
            (first + LAYER_MUTE) as u8
        ]
    );
    host.set(first, 0);
    assert_eq!(host.page(1), (1, 1, vec![first as u8]));
    host.set(first, 3);
    assert_eq!(
        &host.values[controls..controls + BIT_LOGIC.len()],
        &[950, 750, 120, 19, 100, 160, 1000, 6, 1, 3300]
    );
    assert_eq!(host.page(1).1, 15);
}

#[test]
fn common_controls_have_stable_ordinals_at_every_layer_count() {
    let spec = &KickLab::SPECIFICATIONS[0];
    assert_eq!((spec.min, spec.max, spec.default), (1, 8, 1));
    assert!(KickLab::configuration(&[0, DEFAULT_OUTPUT_BUFFERS as i32]).is_none());
    assert!(KickLab::configuration(&[9, DEFAULT_OUTPUT_BUFFERS as i32]).is_none());
    for count in 1..=MAX_LAYERS {
        let mut host = Host::new(count as i32);
        host.step([0.0; 4]);
        for layer in 0..count {
            let base = GLOBALS + 15 * layer;
            let index = base + 1;
            let solo = base + 3;
            let (group, _, indices) = host.page(layer + 1);
            assert_eq!(group, 1);
            assert_eq!(indices[0] as usize, base);
            if layer == 0 {
                assert_eq!(
                    &indices[1..5],
                    &[index as u8, (base + 2) as u8, solo as u8, (solo + 1) as u8]
                );
            } else {
                assert_eq!(indices.len(), 1);
            }
            for (offset, name) in [
                (0, c"Model"),
                (1, c"Randomize"),
                (2, c"Level"),
                (3, c"Solo"),
                (4, c"Mute"),
            ] {
                let def = unsafe { *(*host.base).parameters.add(base + offset) };
                assert_eq!(unsafe { CStr::from_ptr(def.name) }, name);
            }
            let def = unsafe { *(*host.base).parameters.add(index) };
            assert_eq!(unsafe { CStr::from_ptr(def.name) }, c"Randomize");
            assert_eq!(
                (def.min, def.max, def.default, def.unit),
                (0, 1, 0, UNIT_CONFIRM)
            );
            let mut prefix = [0; PARAMETER_UI_PREFIX_SIZE];
            for parameter in [index, solo, solo + 1] {
                let length = unsafe {
                    factory().parameter_ui_prefix.unwrap()(
                        host.base,
                        parameter as i32,
                        prefix.as_mut_ptr(),
                    )
                };
                assert_eq!(length as usize, LAYER_PREFIXES[layer].len());
                assert_eq!(
                    unsafe { CStr::from_ptr(prefix.as_ptr()) }.to_bytes(),
                    LAYER_PREFIXES[layer]
                );
            }
            for (parameter, name) in [(solo, c"Solo"), (solo + 1, c"Mute")] {
                let def = unsafe { *(*host.base).parameters.add(parameter) };
                assert_eq!(unsafe { CStr::from_ptr(def.name) }, name);
                assert_eq!(
                    (def.min, def.max, def.default, def.unit),
                    (0, 1, 0, UNIT_ENUM)
                );
            }
        }
        assert_eq!(host.values.len(), GLOBALS + 15 * count);
    }
}

#[test]
fn confirmed_randomize_runs_once_and_rearms_after_reset() {
    let mut host = Host::uninitialized(MAX_LAYERS as i32);
    let base = model_index(MAX_LAYERS - 1);
    let action = base + LAYER_RANDOMIZE;
    host.values[base] = 3;
    host.values[base + LAYER_LEVEL] = 37;
    host.values[base + LAYER_SOLO] = 1;
    host.values[base + LAYER_MUTE] = 1;
    host.values[action] = 1;
    let original = host.values.clone();
    host.notify(action);
    assert_eq!(host.values[action], 0);
    host.step([0.0; 4]);
    host.complete();
    host.step([0.0; 4]);
    assert_eq!(host.values[action], 0);
    assert_eq!(
        host.values[base + LAYER_LEVEL],
        original[base + LAYER_LEVEL]
    );

    let before = host.values.clone();
    let mut text = [0; PARAMETER_STRING_SIZE];
    unsafe {
        factory().parameter_string.unwrap()(host.base, action as i32, 1, text.as_mut_ptr());
    }
    assert_eq!(unsafe { CStr::from_ptr(text.as_ptr()) }, c"Randomize?");
    assert_eq!(host.values, before);

    for _ in 0..2 {
        let before = host.values.clone();
        let writes = AUDIO_WRITES.with(Cell::get);
        REENTER_RANDOMIZE.with(|reenter| reenter.set(Some(action)));
        host.set(action, 1);
        host.notify(action);
        assert_eq!(AUDIO_WRITES.with(Cell::get) - writes, BIT_LOGIC.len() + 1);
        assert_eq!(host.values[action], 0);
        assert_eq!(
            &host.values[..base + LAYER_FIRST_CONTROL],
            &before[..base + LAYER_FIRST_CONTROL]
        );
        assert_eq!(
            &host.values[base + LAYER_FIELDS..],
            &before[base + LAYER_FIELDS..]
        );
        assert_ne!(
            &host.values[base + LAYER_FIRST_CONTROL..base + LAYER_FIELDS],
            &before[base + LAYER_FIRST_CONTROL..base + LAYER_FIELDS]
        );
        for (offset, control) in BIT_LOGIC.iter().enumerate() {
            assert!((control.range[0]..=control.range[1])
                .contains(&host.values[base + LAYER_FIRST_CONTROL + offset]));
        }
        host.step([0.0; 4]);
        host.complete();
        host.step([0.0; 4]);
        assert!(WORK.with(Cell::get).is_none());
    }

    host.set(base, 0);
    host.step([0.0; 4]);
    host.complete();
    host.step([0.0; 4]);
    let level = host.values[base + LAYER_LEVEL];
    host.set(action, 1);
    host.step([0.0; 4]);
    assert_eq!(host.values[base + LAYER_LEVEL], level);
    assert_eq!(host.values[action], 0);
    assert!(WORK.with(Cell::get).is_none());
}

#[test]
fn confirmed_global_randomize_selects_models_and_clears_layer_switches() {
    CYCLES.with(|cycles| cycles.set(1));
    let mut host = Host::uninitialized(MAX_LAYERS as i32);
    let action = RANDOMIZE_LAYERS;
    assert_eq!(host.page(0), (0, 6, (0..GLOBALS as u8).collect::<Vec<_>>()));
    let def = unsafe { *(*host.base).parameters.add(action) };
    assert_eq!(unsafe { CStr::from_ptr(def.name) }, c"Randomize Layers");
    assert_eq!(
        (def.min, def.max, def.default, def.unit),
        (0, 1, 0, UNIT_CONFIRM)
    );

    host.values[action] = 1;
    host.notify(action);
    assert_eq!(host.values[action], 0);
    assert_eq!(host.values[model_index(0)], 1);

    let mut text = [0; PARAMETER_STRING_SIZE];
    let writes = AUDIO_WRITES.with(Cell::get);
    unsafe {
        factory().parameter_string.unwrap()(host.base, action as i32, 1, text.as_mut_ptr());
    }
    assert_eq!(unsafe { CStr::from_ptr(text.as_ptr()) }, c"Randomize?");
    assert_eq!(AUDIO_WRITES.with(Cell::get), writes);

    host.values[model_index(2) + LAYER_RANDOMIZE] = 1;
    let original = host.values.clone();
    let mut expected_seed = 1;
    for defer in [false, true] {
        for layer in 0..MAX_LAYERS {
            let base = model_index(layer);
            host.values[base + LAYER_SOLO] = 1;
            host.values[base + LAYER_MUTE] = 1;
        }
        DEFER_MODEL_NOTIFICATION.with(|pending| pending.set(defer));
        REENTER_RANDOMIZE.with(|pending| pending.set(Some(action)));
        host.set(action, 1);
        DEFER_MODEL_NOTIFICATION.with(|pending| pending.set(false));
        assert_eq!(host.values[action], 0);
        let writes = AUDIO_WRITES.with(Cell::get);
        host.notify(action);
        assert_eq!(AUDIO_WRITES.with(Cell::get), writes);
        assert_eq!(host.values[..action], original[..action]);

        for layer in 0..MAX_LAYERS {
            let base = model_index(layer);
            let model = random_value(&mut expected_seed, 0, 12);
            let level = random_value(&mut expected_seed, 0, 100);
            assert_eq!(host.values[base], model);
            assert_eq!(host.values[base + LAYER_LEVEL], level);
            assert_eq!(host.values[base + LAYER_SOLO], 0);
            assert_eq!(host.values[base + LAYER_MUTE], 0);
            assert_eq!(
                host.values[base + LAYER_RANDOMIZE],
                original[base + LAYER_RANDOMIZE]
            );
            let controls = Model::from_value(model).controls();
            let visible = if model == 0 {
                1
            } else {
                LAYER_FIRST_CONTROL + controls.len()
            };
            assert_eq!(
                host.page(layer + 1).2,
                (base as u8..(base + visible) as u8).collect::<Vec<_>>()
            );
            for (offset, control) in controls.iter().enumerate() {
                let i = base + LAYER_FIRST_CONTROL + offset;
                let def = unsafe { *(*host.base).parameters.add(i) };
                assert_eq!(unsafe { CStr::from_ptr(def.name) }, control.name);
                let [min, max] = control.range;
                assert_eq!(host.values[i], random_value(&mut expected_seed, min, max));
            }
        }
        host.step([0.0; 4]);
        host.complete();
        host.step([0.0; 4]);
        assert!(WORK.with(Cell::get).is_none());
    }
}

#[test]
fn worker_rejection_retries_and_a_new_hit_overlaps_the_old_version() {
    let mut host = Host::new(1);
    ACCEPT_WORK.with(|accept| accept.set(false));
    assert_eq!(host.step([0.0; 4]), [0.0; 4]);
    assert!(WORK.with(Cell::get).is_none());
    ACCEPT_WORK.with(|accept| accept.set(true));
    host.step([0.0; 4]);
    let first = host.complete();
    let hit = host.step([0.0, 0.0, 5.0, 5.0]);
    assert_eq!(hit, [0.0, 0.0, host.wave(first, 0), host.wave(first, 1)]);

    host.set(model_index(0), 2);
    let tail = host.step([0.0; 4]);
    assert_eq!(
        tail,
        core::array::from_fn(|frame| host.wave(first, frame + 2))
    );
    let second = host.complete();
    assert_ne!(first, second);
    let next_hit = if cfg!(feature = "profiling-hold-playback") {
        first
    } else {
        second
    };
    let overlap = host.step([5.0; 4]);
    assert_eq!(
        overlap,
        core::array::from_fn(|frame| host.wave(first, frame + 6) + host.wave(next_hit, frame))
    );
}

#[test]
fn mix_applies_layer_levels_to_cached_waveforms() {
    let mut host = Host::new(3);
    host.set(model_index(1), 2);
    host.set(model_index(2), 3);
    for layer in 0..3 {
        host.set(model_index(layer) + LAYER_LEVEL, 10);
    }
    for (layer, decay) in [45, 120, 90].into_iter().enumerate() {
        host.set(model_index(layer) + LAYER_FIRST_CONTROL + 1, decay);
    }
    host.step([0.0; 4]);
    let original_mix = host.complete();
    let frame = 761;
    let cached: [f32; 3] = core::array::from_fn(|layer| host.sample(host.layer_slot(layer), frame));
    assert!(cached.iter().all(|&sample| sample != 0.0));
    assert_eq!(
        host.sample(original_mix, frame),
        cached.iter().map(|sample| sample * 0.1).sum::<f32>()
    );
    let modal_tail = host.sample(host.layer_slot(1), 7500) * 0.1;
    assert_ne!(modal_tail, 0.0);
    assert_eq!(host.sample(original_mix, 7500), modal_tail);

    host.set(model_index(0) + LAYER_LEVEL, 0);
    host.set(model_index(1) + LAYER_LEVEL, 20);
    host.step([0.0; 4]);
    let remixed = host.complete();
    assert_ne!(remixed, original_mix);
    assert_eq!(
        host.sample(remixed, frame),
        cached[1] * 0.2 + cached[2] * 0.1
    );
    assert_eq!(
        host.sample(original_mix, frame),
        cached.iter().map(|sample| sample * 0.1).sum::<f32>()
    );
}

#[test]
fn switching_models_reuses_the_layer_cache_and_regenerates_its_contents() {
    let mut host = Host::new(1);
    let model = model_index(0);
    host.set(model, 0);
    host.set(model, 1);
    host.step([0.0; 4]);
    host.complete();
    let prefix_frames = render::layer_cache_frames(48_000) / 2;
    let pitch: Vec<_> = (0..prefix_frames)
        .map(|frame| host.sample(host.layer_slot(0), frame))
        .collect();

    host.set(model, 2);
    host.step([0.0; 4]);
    host.complete();
    assert_ne!(host.sample(host.layer_slot(0), 761), pitch[761]);

    host.set(model, 4);
    host.step([0.0; 4]);
    host.complete();
    assert_ne!(host.sample(host.layer_slot(0), 761), pitch[761]);

    host.set(model, 1);
    host.step([0.0; 4]);
    host.complete();
    for (frame, original) in pitch.into_iter().enumerate() {
        assert_eq!(host.sample(host.layer_slot(0), frame), original);
    }
}

#[test]
fn grain_cloud_fits_the_existing_layer_page_and_restores_its_values() {
    let mut host = Host::uninitialized(4);
    let layer = 3;
    let model = model_index(layer);
    let first_control = model + LAYER_FIRST_CONTROL;
    let density_envelope = first_control + 6;
    assert_eq!(
        unsafe { (*(*host.base).parameters.add(model)).max },
        (MODEL_NAMES.len() - 1) as i16
    );
    host.values[model] = 4;
    host.values[density_envelope] = -25;
    host.notify(model);
    assert_eq!(host.values[density_envelope], -25);
    let mut name = [0; PARAMETER_STRING_SIZE];
    unsafe {
        factory().parameter_string.unwrap()(host.base, model as i32, 4, name.as_mut_ptr());
    }
    assert_eq!(unsafe { CStr::from_ptr(name.as_ptr()) }, c"Grain Cloud");
    let (group, _, indices) = host.page(layer + 1);
    assert_eq!(group, 1);
    assert_eq!(
        indices,
        (model as u8..(first_control + GRAIN_CLOUD.len()) as u8).collect::<Vec<_>>()
    );
    for (offset, control) in GRAIN_CLOUD.iter().enumerate() {
        let parameter = unsafe { *(*host.base).parameters.add(first_control + offset) };
        assert_eq!(unsafe { CStr::from_ptr(parameter.name) }, control.name);
        assert_eq!([parameter.min, parameter.max], control.range);
    }

    host.set(model, 0);
    assert_eq!(host.page(layer + 1), (1, 1, vec![model as u8]));
    host.set(model, 4);
    assert_eq!(
        &host.values[first_control..first_control + GRAIN_CLOUD.len()],
        &[1100, 790, 18, 123, 5, 170, 13, 60, 83, 100]
    );
}

#[test]
fn hard_sync_switches_to_eight_controls_without_moving_mix_or_randomize() {
    let mut host = Host::new(2);
    let layer = 1;
    let model = model_index(layer);
    let solo = model + LAYER_SOLO;
    host.set(model + LAYER_LEVEL, 78);
    host.set(solo, 1);
    host.set(model, 5);
    let first_control = model + LAYER_FIRST_CONTROL;

    let mut name = [0; PARAMETER_STRING_SIZE];
    unsafe {
        factory().parameter_string.unwrap()(host.base, model as i32, 5, name.as_mut_ptr());
    }
    assert_eq!(unsafe { CStr::from_ptr(name.as_ptr()) }, c"Hard Sync");
    assert_eq!(host.values[model + LAYER_LEVEL], 78);
    assert_eq!(host.values[solo], 1);
    assert_eq!(
        host.page(layer + 1),
        (
            1,
            13,
            (model as u8..(model + LAYER_FIRST_CONTROL + HARD_SYNC.len()) as u8).collect()
        )
    );
    for (offset, control) in HARD_SYNC.iter().enumerate() {
        let definition = unsafe { *(*host.base).parameters.add(first_control + offset) };
        assert_eq!(unsafe { CStr::from_ptr(definition.name) }, control.name);
        assert_eq!([definition.min, definition.max], control.range);
    }
    assert_eq!(
        &host.values[first_control..first_control + HARD_SYNC.len()],
        &[950, 725, 36, 160, 17, 100, 220, 150]
    );
    host.step([0.0; 4]);
    let rendered = host.complete();
    assert!(host.sample(rendered, 761).abs() > 0.001);
}

#[test]
fn new_kick_models_use_the_existing_layer_page_and_worker() {
    let mut host = Host::new(4);
    let layer = 3;
    let base = model_index(layer);
    let solo = base + LAYER_SOLO;
    host.set(base + LAYER_LEVEL, 73);
    host.set(solo, 1);
    for (id, name, controls) in [
        (6, c"Resonator", RESONATOR.as_slice()),
        (7, c"Two-Operator FM", TWO_OPERATOR_FM.as_slice()),
        (8, c"Noise Sweep", NOISE_SWEEP.as_slice()),
        (9, c"Ring Mod", RING_MOD.as_slice()),
        (10, c"Wavefold", WAVEFOLD.as_slice()),
        (11, c"Delay Drum", DELAY_DRUM.as_slice()),
        (12, c"Spectral Pulse", SPECTRAL_PULSE.as_slice()),
    ] {
        host.set(base, id);
        let mut text = [0; PARAMETER_STRING_SIZE];
        unsafe {
            factory().parameter_string.unwrap()(
                host.base,
                base as i32,
                i32::from(id),
                text.as_mut_ptr(),
            );
        }
        assert_eq!(unsafe { CStr::from_ptr(text.as_ptr()) }, name);
        let expected: Vec<_> =
            (base as u8..(base + LAYER_FIRST_CONTROL + controls.len()) as u8).collect();
        assert_eq!(host.page(layer + 1), (1, expected.len() as u8, expected));
        assert_eq!(host.values[base + LAYER_LEVEL], 73);
        assert_eq!(host.values[solo], 1);
        for (slot, control) in controls.iter().enumerate() {
            let index = base + LAYER_FIRST_CONTROL + slot;
            let definition = unsafe { *(*host.base).parameters.add(index) };
            assert_eq!(unsafe { CStr::from_ptr(definition.name) }, control.name);
            assert_eq!(host.values[index], control.initial);
        }
        host.step([0.0; 4]);
        let wave = host.complete();
        assert!((1..128).any(|frame| host.sample(wave, frame).abs() > 0.001));
    }
}

#[test]
fn solo_and_mute_select_which_cached_layers_are_mixed() {
    let mut host = Host::new(3);
    host.set(model_index(1), 2);
    for layer in 0..2 {
        host.set(model_index(layer) + LAYER_LEVEL, 10);
    }
    host.step([0.0; 4]);
    let initial = host.complete();
    let frame = 761;
    let first = host.sample(host.layer_slot(0), frame) * 0.1;
    let second = host.sample(host.layer_slot(1), frame) * 0.1;
    assert_ne!(first, 0.0);
    assert_ne!(second, 0.0);
    assert_eq!(host.sample(initial, frame), first + second);

    let page_updates = PAGE_UPDATES.with(Cell::get);
    for (parameter, value, expected) in [
        (model_index(2) + LAYER_SOLO, 1, first + second), // A layer with Model Off cannot solo.
        (model_index(0) + LAYER_SOLO, 1, first),
        (model_index(1) + LAYER_SOLO, 1, first + second),
        (model_index(0) + LAYER_MUTE, 1, second),
        (model_index(0) + LAYER_SOLO, 0, second),
        (model_index(1) + LAYER_SOLO, 0, second),
        (model_index(0) + LAYER_MUTE, 0, first + second),
    ] {
        host.set(parameter, value);
        host.step([0.0; 4]);
        let mix = host.complete();
        assert_eq!(host.sample(mix, frame), expected);
        assert_eq!(host.sample(host.layer_slot(0), frame) * 0.1, first);
        assert_eq!(host.sample(host.layer_slot(1), frame) * 0.1, second);
    }
    assert_eq!(PAGE_UPDATES.with(Cell::get), page_updates);

    host.set(model_index(0) + LAYER_SOLO, 1);
    host.set(model_index(0) + LAYER_MUTE, 1);
    host.step([0.0; 4]);
    host.complete();
    if !cfg!(feature = "profiling-hold-playback") {
        assert_eq!(host.step([5.0; 4]), [0.0; 4]);
    }
}

#[test]
fn one_second_cap_fades_and_finishes_all_models_and_the_filter_tail() {
    for (model, decay, cutoff) in [
        (1, 1416, CUTOFF_OFF),
        (2, 2000, CUTOFF_OFF),
        (3, 2000, CUTOFF_OFF),
        (1, 1416, 0),
        (1, 640, 0),
    ] {
        let mut host = Host::uninitialized(1);
        host.values[model_index(0)] = model;
        host.values[model_index(0) + LAYER_FIRST_CONTROL + 1] = decay;
        host.values[CUTOFF] = cutoff;
        host.notify(model_index(0));
        host.step([0.0; 4]);
        let decay_index = model_index(0) + LAYER_FIRST_CONTROL + 1;
        assert_eq!(host.values[decay_index], decay.min(1000));
        assert_eq!(
            unsafe { (*(*host.base).parameters.add(decay_index)).max },
            1000
        );
        let slot = host.complete();
        assert!((47_760..47_999).any(|frame| host.sample(slot, frame).abs() > 0.000_001));
        assert_eq!(host.sample(slot, 47_999), 0.0);

        host.step([5.0; 4]);
        for _ in 1..11_999 {
            host.step([0.0; 4]);
        }
        assert_eq!(
            host.step([0.0; 4]),
            core::array::from_fn(|frame| host.wave(slot, 47_996 + frame))
        );
        assert_eq!(host.step([0.0; 4]), [0.0; 4]);
    }
}

#[test]
fn single_layer_master_gain_preserves_peak_normalization() {
    let length = 33_360;
    let gain = distingnt_api::api::math::exp2(12.0 * (core::f32::consts::LOG2_10 / 20.0));
    for level in [81, 100] {
        let mut host = Host::new(1);
        host.set(model_index(0) + LAYER_LEVEL, level);
        host.step([0.0; 4]);
        let dry = host.complete();
        let peak = (0..length).fold(1.0f32, |peak, frame| {
            peak.max((host.sample(dry, frame) * gain).abs())
        });
        assert!(peak > 1.0);
        host.set(GAIN, 120);
        host.step([0.0; 4]);
        let boosted = host.complete();
        for frame in 0..length {
            assert_eq!(
                host.sample(boosted, frame),
                host.sample(dry, frame) * gain * peak.recip()
            );
        }
    }
}

#[test]
fn scope_draws_the_published_peak_with_a_fixed_time_axis() {
    let mut host = Host::new(1);
    let draw = || {
        WAVE_SEGMENTS.with(|count| count.set(0));
        WAVE_ENDS.with(|count| count.set(0));
        assert!(!unsafe { factory().draw.unwrap()(host.base) });
        DRAW_LABELS.with_borrow_mut(core::mem::take)
    };
    assert_eq!(draw(), ["Peak: -inf dBFS", "1000 ms"]);
    assert_eq!(WAVE_ENDS.with(Cell::get), 0);
    let base = host.base;
    host.set(GAIN, 120);
    host.step([0.0; 4]);
    host.complete();
    host.step([0.0; 4]);
    assert!(!unsafe { factory().draw.unwrap()(base) });
    assert_eq!(
        DRAW_LABELS.with_borrow_mut(core::mem::take),
        ["Peak: 0.0 dBFS", "1000 ms"]
    );
    assert!(WAVE_SEGMENTS.with(Cell::get) > 0);
    assert!(WAVE_ENDS.with(Cell::get) > 0);
}

#[cfg(feature = "profiling")]
#[test]
fn click_diagnostic_can_suppress_rebuilds_after_the_first_waveform() {
    let mut host = Host::new(1);
    host.step([0.0; 4]);
    host.complete();
    host.step([0.0; 4]);
    host.set(model_index(0) + LAYER_FIRST_CONTROL + 6, 30);
    host.step([0.0; 4]);
    assert_eq!(
        WORK.with(Cell::get).is_none(),
        cfg!(feature = "profiling-no-click-rebuild")
    );
}

#[cfg(feature = "profiling")]
#[test]
fn synthesis_runs_only_for_layers_with_changed_patches() {
    let mut host = Host::new(4);
    host.set(model_index(1), 2);
    host.set(model_index(2), 3);
    host.step([0.0; 4]);
    host.complete();
    for (parameter, value) in [
        (model_index(0) + LAYER_FIRST_CONTROL, 680),
        (CUTOFF, 50),
        (model_index(0) + LAYER_LEVEL, 75),
        (GAIN, -50),
    ] {
        host.set(parameter, value);
        host.step([0.0; 4]);
        host.complete();
    }
    for _ in 0..12_040 {
        host.step([0.0; 4]);
    }
    PROFILE_PACKETS.with(|packets| {
        let received = packets.borrow();
        let packets: Vec<_> = received
            .iter()
            .filter(|packet| packet.get(2) == Some(&b'N'))
            .collect();
        let first_metric = profile::FUNCTIONS.len() + 1;
        assert_eq!(packets.len(), first_metric + profile::FUNCTIONS.len());
        let decode = |bytes: &[u8]| {
            bytes
                .iter()
                .rev()
                .fold(0, |sum, &digit| (sum << 7) | u64::from(digit))
        };
        for (id, (packet, calls)) in packets[first_metric..]
            .iter()
            .zip([
                Some(12_000),
                Some(12_000),
                Some(5),
                None,
                Some(4),
                Some(3),
                Some(5),
                Some(if cfg!(feature = "profiling-hold-playback") {
                    1
                } else {
                    5
                }),
                Some(5),
                Some(11_999),
                Some(11_999),
                None,
            ])
            .enumerate()
        {
            assert_eq!(decode(&packet[24..26]), id as u64);
            if let Some(calls) = calls {
                assert_eq!(decode(&packet[41..46]), calls);
            }
        }
        let total = |id: usize| decode(&packets[first_metric + id][46..56]);
        assert!(total(2) >= total(4) + total(5) + total(6));
        assert_eq!(total(4), 52); // Three initial layers, then only the edited layer.
        assert_eq!(total(5), 39);
        assert_eq!(total(6), 65);
    });
}
