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
extern "C" fn NT_setParameterFromAudio(algorithm: u32, parameter: u32, value: i16) {
    assert_eq!(algorithm, 3);
    AUDIO_WRITES.with(|writes| writes.set(writes.get() + 1));
    let base = ACTIVE.with(Cell::get);
    let local = (parameter - NT_parameterOffset()) as usize;
    assert!(!matches!(
        parameter_layer(local as i32, MAX_LAYERS),
        Some((_, 0))
    ));
    unsafe {
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
        factory().parameter_changed.unwrap()(base, local as i32);
    }
}

#[no_mangle]
extern "C" fn NT_updateParameterDefinition(algorithm: u32, local_parameter: u32) {
    assert_eq!(algorithm, 3);
    assert!(matches!(
        parameter_layer(local_parameter as i32, MAX_LAYERS),
        Some((_, 2..))
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
        initialize_factory();
        let mut req = NtAlgorithmRequirements {
            num_parameters: 0,
            sram: 0,
            dram: 0,
            dtc: 0,
            itc: 0,
        };
        unsafe { factory().calculate_requirements.unwrap()(&mut req, &layers) };
        assert_eq!(req.num_parameters as usize, solo_index(layers as usize));
        assert_eq!(
            req.dram as usize,
            ((BUFFERS + layers as usize) * render::max_frames(48_000)
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
        let base = unsafe { factory().construct.unwrap()(&mem, &req, &layers) };
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
            .map(|index| KickLab::dram_buffer_len_at(&[], 48_000, index))
            .sum();
        self.dram[start + frame]
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
    host.values[first + 2 + 5] = 415; // Outside the Pitch Sweep slot's range.
    host.values[first + 2 + 8] = 2; // OR
    host.values[first + 1] = 72;
    host.notify(first);
    assert_eq!(host.values[first + 7], 415);
    assert_eq!(host.values[first + 10], 2);
    assert_eq!(
        host.page(1),
        (
            1,
            15,
            [
                first as u8,
                (first + 1) as u8,
                MIX_START as u8,
                (MIX_START + 1) as u8
            ]
            .into_iter()
            .chain((first + 2) as u8..(first + 12) as u8)
            .chain([RANDOMIZE_START as u8])
            .collect()
        )
    );
    assert_eq!(host.page(2), (1, 1, vec![second as u8]));
    assert_eq!(PAGE_UPDATES.with(Cell::get), 1);

    host.set(first, 2);
    assert_eq!(host.values[first], 2);
    assert_eq!(host.values[first + 1], 72);
    assert_eq!(host.values[first + 2], 570);
    assert_eq!(host.values[first + 7], 50);
    assert_eq!(host.page(1).1, 13);
    assert_eq!(host.page(1).2.last(), Some(&(RANDOMIZE_START as u8)));
    host.set(first, 0);
    assert_eq!(host.page(1), (1, 1, vec![first as u8]));
    host.set(first, 3);
    assert_eq!(host.values[first + 7], 205);
    assert_eq!(host.page(1).1, 15);
}

#[test]
fn mix_switches_and_randomize_have_stable_ordinals_at_every_layer_count() {
    for count in 1..=MAX_LAYERS {
        let mut host = Host::new(count as i32);
        host.step([0.0; 4]);
        for layer in 0..count {
            let index = RANDOMIZE_START + layer;
            let solo = MIX_START + 2 * layer;
            let (group, _, indices) = host.page(layer + 1);
            assert_eq!(group, 1);
            assert_eq!(indices[0] as usize, model_index(layer));
            if layer == 0 {
                assert_eq!(
                    &indices[1..4],
                    &[(model_index(layer) + 1) as u8, solo as u8, (solo + 1) as u8]
                );
                assert_eq!(indices.last().copied(), Some(index as u8));
            } else {
                assert_eq!(indices.len(), 1);
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
        for index in (model_index(count)..RANDOMIZE_START).chain(RANDOMIZE_START + count..MIX_START)
        {
            let def = unsafe { *(*host.base).parameters.add(index) };
            assert_eq!(unsafe { CStr::from_ptr(def.name) }, c"Unused");
        }
    }
}

#[test]
fn confirmed_randomize_runs_once_and_rearms_after_reset() {
    let mut host = Host::uninitialized(2);
    let base = model_index(1);
    let action = RANDOMIZE_START + 1;
    host.values[base] = 3;
    host.values[base + 1] = 37;
    host.values[solo_index(1)] = 1;
    host.values[solo_index(1) + 1] = 1;
    host.values[action] = 1;
    let original = host.values.clone();
    host.notify(action);
    assert_eq!(host.values[action], 0);
    host.step([0.0; 4]);
    host.complete();
    host.step([0.0; 4]);
    assert_eq!(host.values[action], 0);
    assert_eq!(host.values[base + 1], original[base + 1]);

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
        assert_eq!(&host.values[..base + 2], &before[..base + 2]);
        assert_eq!(
            &host.values[base + LAYER_FIELDS..],
            &before[base + LAYER_FIELDS..]
        );
        assert_ne!(
            &host.values[base + 2..base + LAYER_FIELDS],
            &before[base + 2..base + LAYER_FIELDS]
        );
        for (offset, control) in BIT_LOGIC.iter().enumerate() {
            assert!((control.range[0]..=control.range[1]).contains(&host.values[base + 2 + offset]));
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
    let level = host.values[base + 1];
    host.set(action, 1);
    host.step([0.0; 4]);
    assert_eq!(host.values[base + 1], level);
    assert_eq!(host.values[action], 0);
    assert!(WORK.with(Cell::get).is_none());
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
        host.set(model_index(layer) + 1, 10);
    }
    for (layer, decay) in [45, 120, 90].into_iter().enumerate() {
        host.set(model_index(layer) + 3, decay);
    }
    host.step([0.0; 4]);
    let original_mix = host.complete();
    let frame = 761;
    let cached: [f32; 3] = core::array::from_fn(|layer| host.sample(layer_slot(layer), frame));
    assert!(cached.iter().all(|&sample| sample != 0.0));
    assert_eq!(
        host.sample(original_mix, frame),
        cached.iter().map(|sample| sample * 0.1).sum::<f32>()
    );
    let modal_tail = host.sample(layer_slot(1), 7500) * 0.1;
    assert_ne!(modal_tail, 0.0);
    assert_eq!(host.sample(original_mix, 7500), modal_tail);

    host.set(model_index(0) + 1, 0);
    host.set(model_index(1) + 1, 20);
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
    host.step([0.0; 4]);
    host.complete();
    let prefix_frames = render::layer_cache_frames(48_000) / 2;
    let pitch: Vec<_> = (0..prefix_frames)
        .map(|frame| host.sample(layer_slot(0), frame))
        .collect();

    host.set(model, 2);
    host.step([0.0; 4]);
    host.complete();
    assert_ne!(host.sample(layer_slot(0), 761), pitch[761]);

    host.set(model, 4);
    host.step([0.0; 4]);
    host.complete();
    assert_ne!(host.sample(layer_slot(0), 761), pitch[761]);

    host.set(model, 1);
    host.step([0.0; 4]);
    host.complete();
    for (frame, original) in pitch.into_iter().enumerate() {
        assert_eq!(host.sample(layer_slot(0), frame), original);
    }
}

#[test]
fn grain_cloud_fits_the_existing_layer_page_and_restores_its_values() {
    let mut host = Host::uninitialized(4);
    let layer = 3;
    let model = model_index(layer);
    let first_control = model + 2;
    let density_envelope = first_control + 6;
    assert_eq!(unsafe { (*(*host.base).parameters.add(model)).max }, 4);
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
        [model, model + 1, solo_index(layer), solo_index(layer) + 1]
            .into_iter()
            .chain(first_control..first_control + GRAIN_CLOUD.len())
            .chain([RANDOMIZE_START + layer])
            .map(|index| index as u8)
            .collect::<Vec<_>>()
    );
    for (offset, control) in GRAIN_CLOUD.iter().enumerate() {
        let parameter = unsafe { *(*host.base).parameters.add(first_control + offset) };
        assert_eq!(unsafe { CStr::from_ptr(parameter.name) }, control.name);
        assert_eq!([parameter.min, parameter.max], control.range);
    }

    host.set(model, 0);
    assert_eq!(host.page(layer + 1), (1, 1, vec![model as u8]));
    host.set(model, 4);
    for (offset, control) in GRAIN_CLOUD.iter().enumerate() {
        assert_eq!(host.values[first_control + offset], control.initial);
    }
}

#[test]
fn solo_and_mute_select_which_cached_layers_are_mixed() {
    let mut host = Host::new(3);
    host.set(model_index(1), 2);
    for layer in 0..2 {
        host.set(model_index(layer) + 1, 10);
    }
    host.step([0.0; 4]);
    let initial = host.complete();
    let frame = 761;
    let first = host.sample(layer_slot(0), frame) * 0.1;
    let second = host.sample(layer_slot(1), frame) * 0.1;
    assert_ne!(first, 0.0);
    assert_ne!(second, 0.0);
    assert_eq!(host.sample(initial, frame), first + second);

    let page_updates = PAGE_UPDATES.with(Cell::get);
    for (parameter, value, expected) in [
        (solo_index(2), 1, first + second), // A layer with Model Off cannot solo.
        (solo_index(0), 1, first),
        (solo_index(1), 1, first + second),
        (solo_index(0) + 1, 1, second),
        (solo_index(0), 0, second),
        (solo_index(1), 0, second),
        (solo_index(0) + 1, 0, first + second),
    ] {
        host.set(parameter, value);
        host.step([0.0; 4]);
        let mix = host.complete();
        assert_eq!(host.sample(mix, frame), expected);
        assert_eq!(host.sample(layer_slot(0), frame) * 0.1, first);
        assert_eq!(host.sample(layer_slot(1), frame) * 0.1, second);
    }
    assert_eq!(PAGE_UPDATES.with(Cell::get), page_updates);

    host.set(solo_index(0), 1);
    host.set(solo_index(0) + 1, 1);
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
        host.values[model_index(0) + 3] = decay;
        host.values[CUTOFF] = cutoff;
        host.notify(model_index(0));
        host.step([0.0; 4]);
        let decay_index = model_index(0) + 3;
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
        host.set(model_index(0) + 1, level);
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

#[cfg(feature = "profiling")]
#[test]
fn click_diagnostic_can_suppress_rebuilds_after_the_first_waveform() {
    let mut host = Host::new(1);
    host.step([0.0; 4]);
    host.complete();
    host.step([0.0; 4]);
    host.set(model_index(0) + 2 + 6, 30);
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
        (model_index(0) + 2, 680),
        (CUTOFF, 50),
        (model_index(0) + 1, 75),
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
