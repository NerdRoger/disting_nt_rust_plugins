//! Layered mono kick drum with confirmed model selection and rendered hits.
#![no_std]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod lookup;
mod playback;
#[cfg(feature = "profiling")]
mod profile;
mod render;
#[cfg(not(feature = "profiling"))]
mod profile {
    #[derive(Clone, Copy)]
    pub struct Meter;
    impl Meter {
        pub const EMPTY: Self = Self;
    }
    pub type RenderTiming = ();
    pub type GrainTiming = ();
    pub const NO_RENDER_TIMING: RenderTiming = ();
}

#[cfg(feature = "profiling")]
use distingnt_api::api::cpu_cycle_count;

use core::ffi::{c_char, CStr};
use distingnt_api::{
    api::{
        audio::bus_index,
        background::DramBuffers,
        metadata::{Definition, EnumStrings},
        parameters::{scaled_value, Audio, ReadOnly},
        plugin::{
            Configuration, Context, Metadata, Plugin, RegionInitializer, SharedResources,
            SharedStaticDram, Specification, StaticDramView, StaticMemory, StaticRequirements,
        },
        state_memory::Sram,
    },
    *,
};
use playback::Player;

const MAX_LAYERS: usize = 4;
const MIN_OUTPUT_BUFFERS: usize = 2;
const DEFAULT_OUTPUT_BUFFERS: usize = 6;
const MAX_OUTPUT_BUFFERS: usize = 10;
const TRIGGER: usize = 0;
const OUTPUT: usize = 1;
const OUTPUT_MODE: usize = 2;
const GAIN: usize = 3;
const CUTOFF: usize = 4;
const CUTOFF_OFF: i16 = 101;
const RANDOMIZE_LAYERS: usize = 5;
const GLOBALS: usize = 6;
const CONTROLS: usize = 10;
// Each layer has a fixed-width parameter block in page order.
const LAYER_RANDOMIZE: usize = 1;
const LAYER_LEVEL: usize = 2;
const LAYER_SOLO: usize = 3;
const LAYER_MUTE: usize = 4;
const LAYER_FIRST_CONTROL: usize = 5;
const LAYER_FIELDS: usize = LAYER_FIRST_CONTROL + CONTROLS;
const LAYER_NAMES: [&CStr; MAX_LAYERS] = [c"Layer 1", c"Layer 2", c"Layer 3", c"Layer 4"];
const LAYER_PREFIXES: [&[u8]; MAX_LAYERS] = [b"L1: ", b"L2: ", b"L3: ", b"L4: "];
const MODEL_NAMES: [&CStr; 13] = [
    c"Off",
    c"Pitch Sweep",
    c"Modal Drum",
    c"Bit Logic",
    c"Grain Cloud",
    c"Hard Sync",
    c"Resonator",
    c"Two-Operator FM",
    c"Noise Sweep",
    c"Ring Mod",
    c"Wavefold",
    c"Delay Drum",
    c"Spectral Pulse",
];
static OPERATIONS: EnumStrings<4> = EnumStrings::new([c"XOR", c"AND", c"OR", c"Wrap Add"]);
static OFF_ON: EnumStrings<2> = EnumStrings::new([c"Off", c"On"]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Model {
    Off,
    PitchSweep,
    ModalDrum,
    BitLogic,
    GrainCloud,
    HardSync,
    Resonator,
    TwoOperatorFm,
    NoiseSweep,
    RingMod,
    Wavefold,
    DelayDrum,
    SpectralPulse,
}

impl Model {
    fn from_value(value: i16) -> Self {
        match value {
            1 => Self::PitchSweep,
            2 => Self::ModalDrum,
            3 => Self::BitLogic,
            4 => Self::GrainCloud,
            5 => Self::HardSync,
            6 => Self::Resonator,
            7 => Self::TwoOperatorFm,
            8 => Self::NoiseSweep,
            9 => Self::RingMod,
            10 => Self::Wavefold,
            11 => Self::DelayDrum,
            12 => Self::SpectralPulse,
            _ => Self::Off,
        }
    }

    fn controls(self) -> &'static [Control] {
        match self {
            Self::Off => &[],
            Self::PitchSweep => &PITCH_SWEEP,
            Self::ModalDrum => &MODAL_DRUM,
            Self::BitLogic => &BIT_LOGIC,
            Self::GrainCloud => &GRAIN_CLOUD,
            Self::HardSync => &HARD_SYNC,
            Self::Resonator => &RESONATOR,
            Self::TwoOperatorFm => &TWO_OPERATOR_FM,
            Self::NoiseSweep => &NOISE_SWEEP,
            Self::RingMod => &RING_MOD,
            Self::Wavefold => &WAVEFOLD,
            Self::DelayDrum => &DELAY_DRUM,
            Self::SpectralPulse => &SPECTRAL_PULSE,
        }
    }
}

const fn definition(
    name: &'static CStr,
    min: i16,
    max: i16,
    initial: i16,
    unit: u8,
    scale: u8,
) -> Definition {
    Definition::new(name, min, max, initial, unit, scale, None).unwrap()
}

#[derive(Clone, Copy)]
struct Control {
    name: &'static CStr,
    range: [i16; 2],
    initial: i16,
    unit: u8,
    scale: u8,
}

impl Control {
    const fn new(
        name: &'static CStr,
        min: i16,
        max: i16,
        initial: i16,
        unit: u8,
        scale: u8,
    ) -> Self {
        Self {
            name,
            range: [min, max],
            initial,
            unit,
            scale,
        }
    }

    fn definition(self) -> Option<Definition> {
        let enums = (self.unit == UNIT_ENUM).then(|| OPERATIONS.table());
        Definition::new(
            self.name,
            self.range[0],
            self.range[1],
            self.initial,
            self.unit,
            self.scale,
            enums,
        )
    }

    fn decode(self, value: i16) -> f32 {
        scaled_value(value.max(self.range[0]).min(self.range[1]), self.scale)
    }
}

const fn tune(name: &'static CStr, initial: i16) -> Control {
    Control::new(name, 250, 1200, initial, UNIT_HZ, SCALING_10)
}
const fn decay(initial: i16) -> Control {
    Control::new(c"Body Decay", 45, 1000, initial, UNIT_MS, SCALING_NONE)
}
const fn drive(initial: i16) -> Control {
    Control::new(c"Drive", 0, 240, initial, UNIT_DB, SCALING_10)
}
const fn depth(initial: i16) -> Control {
    Control::new(c"Sweep Depth", 0, 60, initial, UNIT_SEMITONES, SCALING_NONE)
}
const fn time(initial: i16) -> Control {
    Control::new(c"Sweep Time", 1, 200, initial, UNIT_MS, SCALING_NONE)
}
const fn percent(name: &'static CStr, initial: i16) -> Control {
    Control::new(name, 0, 100, initial, UNIT_PERCENT, SCALING_NONE)
}

const PITCH_CLICK_MAX_DECAY_TENTHS_MS: i16 = 450;
const PITCH_SWEEP: [Control; 9] = [
    tune(c"Tail Tune", 500),
    decay(460),
    drive(20),
    depth(24),
    time(24),
    Control::new(c"Sweep Curve", -100, 100, 0, UNIT_PERCENT, SCALING_NONE),
    percent(c"Click Level", 24),
    Control::new(c"Click Tone", 1100, 10_000, 5500, UNIT_HZ, SCALING_NONE),
    Control::new(
        c"Click Decay",
        10,
        PITCH_CLICK_MAX_DECAY_TENTHS_MS,
        60,
        UNIT_MS,
        SCALING_10,
    ),
];
const FIRST_LAYER_PITCH_SWEEP: [i16; PITCH_SWEEP.len()] = [
    900,  // Tail Tune
    500,  // Body Decay
    164,  // Drive
    10,   // Sweep Depth
    50,   // Sweep Time
    15,   // Sweep Curve
    20,   // Click Level
    3000, // Click Tone
    160,  // Click Decay
];
const MODAL_DRUM: [Control; 8] = [
    tune(c"Tail Tune", 570),
    decay(350),
    drive(10),
    percent(c"Upper Modes", 62),
    percent(c"Damping", 44),
    percent(c"Strike Position", 50),
    percent(c"Beater Level", 60),
    percent(c"Beater Hardness", 45),
];
const BIT_LOGIC: [Control; 10] = [
    tune(c"Base Tune", 480),
    decay(370),
    drive(30),
    depth(25),
    time(32),
    Control::new(c"Ramp Ratio", 100, 600, 205, UNIT_NONE, SCALING_100),
    Control::new(c"Sample Clock", 500, 15_000, 3560, UNIT_HZ, SCALING_NONE),
    Control::new(c"Ramp Bits", 3, 12, 5, UNIT_NONE, SCALING_NONE),
    Control::new(c"Logic Operation", 0, 3, 0, UNIT_ENUM, SCALING_NONE),
    Control::new(c"Tone Cutoff", 160, 8500, 1600, UNIT_HZ, SCALING_NONE),
];
const GRAIN_SIZE_MAX_MS: i16 = 100;
const GRAIN_CLOUD: [Control; 10] = [
    tune(c"Tail Tune", 550),
    decay(470),
    depth(20),
    time(70),
    Control::new(
        c"Grain Size",
        5,
        GRAIN_SIZE_MAX_MS,
        29,
        UNIT_MS,
        SCALING_NONE,
    ),
    Control::new(c"Grain Rate", 18, 220, 107, UNIT_HZ, SCALING_NONE),
    Control::new(c"Density Envelope", -30, 30, 0, UNIT_NONE, SCALING_10),
    Control::new(c"Pitch Spread", 0, 240, 48, UNIT_SEMITONES, SCALING_10),
    percent(c"Timing Jitter", 35),
    drive(40),
];
const HARD_SYNC: [Control; 8] = [
    tune(c"Tail Tune", 510),
    decay(410),
    depth(21),
    time(34),
    Control::new(c"Sync Bend", 0, 120, 55, UNIT_NONE, SCALING_10),
    Control::new(c"Sync Decay", 5, 500, 110, UNIT_MS, SCALING_NONE),
    Control::new(c"Tail Ratio", 100, 600, 100, UNIT_NONE, SCALING_100),
    drive(50),
];
const RESONATOR: [Control; 9] = [
    tune(c"Tail Tune", 530),
    decay(820),
    depth(13),
    time(12),
    Control::new(c"Tail Pitch Drift", 0, 60, 15, UNIT_SEMITONES, SCALING_10),
    percent(c"Excitation", 65),
    Control::new(c"Exciter Width", 20, 300, 50, UNIT_MS, SCALING_100),
    Control::new(c"Tone Cutoff", 180, 8000, 2000, UNIT_HZ, SCALING_NONE),
    drive(10),
];
const TWO_OPERATOR_FM: [Control; 9] = [
    tune(c"Tail Tune", 490),
    decay(420),
    depth(24),
    time(22),
    Control::new(c"FM Amount", 0, 900, 320, UNIT_NONE, SCALING_100),
    Control::new(c"Modulator Ratio", 50, 1600, 345, UNIT_NONE, SCALING_100),
    Control::new(c"FM Decay", 2, 400, 78, UNIT_MS, SCALING_NONE),
    Control::new(c"Modulator Sweep", -24, 36, 0, UNIT_SEMITONES, SCALING_NONE),
    drive(30),
];
const NOISE_SWEEP: [Control; 8] = [
    tune(c"Tail Center", 580),
    decay(260),
    depth(41),
    time(27),
    Control::new(c"Filter Q", 5, 120, 41, UNIT_NONE, SCALING_10),
    Control::new(c"Q Sweep", -30, 30, 0, UNIT_NONE, SCALING_10),
    percent(c"Grain", 12),
    drive(40),
];
const RING_MOD: [Control; 9] = [
    tune(c"Tail Tune", 520),
    decay(420),
    depth(17),
    time(32),
    Control::new(c"Lower Osc Ratio", 50, 900, 350, UNIT_NONE, SCALING_100),
    percent(c"Clang Level", 65),
    Control::new(c"Clang Decay", 2, 600, 102, UNIT_MS, SCALING_NONE),
    Control::new(c"Clang Pitch Env", -24, 24, 0, UNIT_SEMITONES, SCALING_NONE),
    drive(30),
];
const WAVEFOLD: [Control; 9] = [
    tune(c"Tail Tune", 490),
    decay(430),
    depth(16),
    time(28),
    Control::new(c"Fold Amount", 0, 100, 42, UNIT_NONE, SCALING_10),
    Control::new(c"Fold Decay", 2, 650, 158, UNIT_MS, SCALING_NONE),
    Control::new(c"Fold Bias", -80, 80, 26, UNIT_PERCENT, SCALING_NONE),
    percent(c"Clean Sub Blend", 0),
    drive(10),
];
const DELAY_DRUM: [Control; 7] = [
    tune(c"Tail Tune", 640),
    decay(520),
    Control::new(c"Loop Cutoff", 250, 8500, 1700, UNIT_HZ, SCALING_NONE),
    Control::new(c"Strike Mix", 0, 200, 100, UNIT_PERCENT, SCALING_NONE),
    Control::new(c"Strike Width", 5, 200, 57, UNIT_MS, SCALING_10),
    Control::new(c"Strike Tone", 200, 8500, 2600, UNIT_HZ, SCALING_NONE),
    drive(50),
];
const SPECTRAL_PULSE: [Control; 7] = [
    tune(c"Tail Tune", 540),
    Control::new(c"Body Decay", 70, 850, 345, UNIT_MS, SCALING_NONE),
    percent(c"Snap Level", 75),
    Control::new(c"Snap Center", 180, 3800, 720, UNIT_HZ, SCALING_NONE),
    Control::new(c"Snap Bandwidth", 5, 100, 38, UNIT_PERCENT, SCALING_NONE),
    Control::new(c"Dispersion", 0, 200, 12, UNIT_MS, SCALING_10),
    drive(20),
];
const UNUSED: Definition = definition(c"Unused", 0, 0, 0, UNIT_NONE, SCALING_NONE);
const LEVEL: Control = percent(c"Level", 100);
const RANDOMIZE: Definition = definition(c"Randomize", 0, 1, 0, UNIT_CONFIRM, SCALING_NONE);

#[derive(Clone, Copy, PartialEq)]
struct LayerPatch {
    model: Model,
    controls: [f32; CONTROLS],
}
impl LayerPatch {
    const OFF: Self = Self {
        model: Model::Off,
        controls: [0.0; CONTROLS],
    };
}

#[derive(Clone, Copy)]
struct Layer {
    patch: LayerPatch,
    level: f32,
    solo: bool,
    mute: bool,
}
impl Layer {
    const OFF: Self = Self {
        patch: LayerPatch::OFF,
        level: 0.0,
        solo: false,
        mute: false,
    };

    fn audible(&self, solo_active: bool) -> bool {
        self.patch.model != Model::Off
            && !self.mute
            && (!solo_active || self.solo)
            && self.level != 0.0
    }
}

#[derive(Clone, Copy)]
struct Job {
    layers: [Layer; MAX_LAYERS],
    rate: u32,
    gain: f32,
    cutoff: i16,
    output_buffers: u8,
}
impl Job {
    const EMPTY: Self = Self {
        layers: [Layer::OFF; MAX_LAYERS],
        rate: 0,
        gain: 0.0,
        cutoff: 0,
        output_buffers: DEFAULT_OUTPUT_BUFFERS as u8,
    };

    fn initialize(destination: &mut RegionInitializer<'_, Self>) {
        distingnt_api::initialize_region!(
            destination,
            Job {
                rate: 0,
                gain: 0.0,
                cutoff: 0,
                output_buffers: DEFAULT_OUTPUT_BUFFERS as u8,
            },
            arrays {
                layers: |_| Layer::OFF
            }
        );
    }
}

#[derive(Clone, Copy)]
struct RenderWork {
    job: Job,
    // The worker alone uses these keys and the per-layer DRAM slots.
    cached_layers: [render::CachedLayer; MAX_LAYERS],
    length: Option<usize>,
    timing: profile::RenderTiming,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParameterSetup {
    New,
    Restoring,
    Ready,
}

#[derive(Clone, Copy)]
struct State {
    layers: usize,
    models: [Model; MAX_LAYERS],
    rate: u32,
    parameter_setup: ParameterSetup,
    dirty: bool,
    randomizing: u8,
    output_buffers: u8,
    random_seed: u32,
    in_flight: Option<usize>,
    player: Player,
    #[cfg_attr(not(feature = "profiling"), allow(dead_code))]
    meter: profile::Meter,
}

struct KickLab;
static TABLE_DRAM: SharedStaticDram<KickLab> = SharedStaticDram::new();
type WaveBuffers = DramBuffers<f32, { MAX_OUTPUT_BUFFERS + 2 * MAX_LAYERS }>;
type Resources = SharedResources<RenderWork, (), WaveBuffers>;
type Callback<'a, Mode> = Context<'a, State, (), Mode, Sram, Resources>;

fn layer_slot(output_buffers: usize, layer: usize) -> usize {
    output_buffers + 2 * layer
}

fn layer_count(specs: &[i32]) -> Option<usize> {
    let count = usize::try_from(*specs.first()?).ok()?;
    (1..=MAX_LAYERS).contains(&count).then_some(count)
}

fn output_buffer_count(specs: &[i32]) -> Option<usize> {
    let count = usize::try_from(*specs.get(1)?).ok()?;
    (MIN_OUTPUT_BUFFERS..=MAX_OUTPUT_BUFFERS)
        .contains(&count)
        .then_some(count)
}

fn model_index(layer: usize) -> usize {
    GLOBALS + LAYER_FIELDS * layer
}

fn layer_page_indices(layer: usize) -> [u8; LAYER_FIELDS] {
    let base = model_index(layer);
    core::array::from_fn(|slot| (base + slot) as u8)
}

fn cutoff_hz(value: i16) -> i16 {
    if value >= CUTOFF_OFF {
        return 0;
    }
    // 101 logarithmic steps from 40 Hz to 20 kHz, followed by Off.
    let exponent = value.max(0) as f32 * (8.965_784 / 100.0);
    (40.0 * distingnt_api::api::math::exp2(exponent) + 0.5) as i16
}

fn parameter_layer(parameter: i32, layers: usize) -> Option<(usize, usize)> {
    let offset = usize::try_from(parameter).ok()?.checked_sub(GLOBALS)?;
    let layer = offset / LAYER_FIELDS;
    (layer < layers).then_some((layer, offset % LAYER_FIELDS))
}

fn update_model_controls(context: &mut Callback<'_, Audio>, layer: usize, restoring: bool) {
    let base = model_index(layer);
    let model = Model::from_value(context.read_parameters().value(base).unwrap_or(0));
    let Some(previous) = context.state_mut().models.get_mut(layer) else {
        return;
    };
    if !restoring && model == *previous {
        return;
    }
    *previous = model;
    let controls = model.controls();
    for slot in 0..CONTROLS {
        let index = base + LAYER_FIRST_CONTROL + slot;
        let control = controls.get(slot).copied();
        let def = control.and_then(Control::definition).unwrap_or(UNUSED);
        // Read saved controls before the host can clamp them to the new definition.
        let value = control.map_or(0, |control| {
            if restoring {
                let [min, max] = control.range;
                context
                    .read_parameters()
                    .value(index)
                    .unwrap_or(0)
                    .max(min)
                    .min(max)
            } else {
                control.initial
            }
        });
        context.parameters().replace_definition(index, def);
        if (!restoring || control.is_some())
            && context.read_parameters().value(index) != Some(value)
        {
            context.parameters().set(index, value);
        }
    }
    let indices = layer_page_indices(layer);
    let visible = if model == Model::Off {
        1
    } else {
        LAYER_FIRST_CONTROL + controls.len()
    };
    if let Some(indices) = indices.get(..visible) {
        context.set_page_parameter_indices(layer + 1, indices);
    }
}

fn restore_model_controls(context: &mut Callback<'_, Audio>) {
    // The host has populated its value array before notifying the plugin.
    // Ignore nested notifications while publishing the restored definitions.
    context.state_mut().parameter_setup = ParameterSetup::Restoring;
    for layer in 0..context.state().layers {
        update_model_controls(context, layer, true);
        // A nonzero action restored with a preset is a stale request.
        let action = model_index(layer) + LAYER_RANDOMIZE;
        if context.read_parameters().value(action) == Some(1) {
            context.parameters().set(action, 0);
        }
    }
    if context.read_parameters().value(RANDOMIZE_LAYERS) == Some(1) {
        context.parameters().set(RANDOMIZE_LAYERS, 0);
    }
    let state = context.state_mut();
    state.parameter_setup = ParameterSetup::Ready;
    state.dirty = true;
}

fn random_value(seed: &mut u32, min: i16, max: i16) -> i16 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    let width = (i32::from(max) - i32::from(min) + 1) as u32;
    let offset = ((u64::from(*seed) * u64::from(width)) >> 32) as i32;
    (i32::from(min) + offset) as i16
}

fn randomize_model_controls(context: &mut Callback<'_, Audio>, base: usize, model: Model) {
    for (index, control) in model.controls().iter().enumerate() {
        let [min, max] = control.range;
        let value = random_value(&mut context.state_mut().random_seed, min, max);
        context
            .parameters()
            .set(base + LAYER_FIRST_CONTROL + index, value);
    }
}

fn randomize_layer(context: &mut Callback<'_, Audio>, layer: usize) {
    let base = model_index(layer);
    let action = base + LAYER_RANDOMIZE;
    let request = 1 << layer;
    if context.state().randomizing & request != 0
        || context.read_parameters().value(action) != Some(1)
    {
        return;
    }
    let Some(&model) = context.state().models.get(layer) else {
        return;
    };
    context.state_mut().randomizing |= request;
    if model != Model::Off {
        randomize_model_controls(context, base, model);
        context.state_mut().dirty = true;
    }
    // Keep the same-layer guard set until the host's reset notification returns.
    context.parameters().set(action, 0);
    context.state_mut().randomizing &= !request;
}

fn randomize_layers(context: &mut Callback<'_, Audio>) {
    let request = 1 << MAX_LAYERS;
    if context.state().randomizing & request != 0
        || context.read_parameters().value(RANDOMIZE_LAYERS) != Some(1)
    {
        return;
    }
    context.state_mut().randomizing |= request;
    for layer in 0..context.state().layers {
        let base = model_index(layer);
        let model_value = random_value(
            &mut context.state_mut().random_seed,
            0,
            (MODEL_NAMES.len() - 1) as i16,
        );
        if context.read_parameters().value(base) != Some(model_value) {
            context.parameters().set(base, model_value);
            // Refresh definitions even when the host defers its notification.
            update_model_controls(context, layer, false);
        }
        let [min, max] = LEVEL.range;
        let level = random_value(&mut context.state_mut().random_seed, min, max);
        context.parameters().set(base + LAYER_LEVEL, level);
        for offset in [LAYER_SOLO, LAYER_MUTE] {
            let parameter = base + offset;
            if context.read_parameters().value(parameter) == Some(1) {
                context.parameters().set(parameter, 0);
            }
        }
        if let Some(&model) = context.state().models.get(layer) {
            randomize_model_controls(context, base, model);
        }
    }
    context.state_mut().dirty = true;
    context.parameters().set(RANDOMIZE_LAYERS, 0);
    context.state_mut().randomizing &= !request;
}

fn handle_parameter_change(context: &mut Callback<'_, Audio>, parameter: i32) {
    match context.state().parameter_setup {
        ParameterSetup::New => {
            if context.read_parameters().value(TRIGGER).is_some() {
                restore_model_controls(context);
            }
            return;
        }
        ParameterSetup::Restoring => return,
        ParameterSetup::Ready => {}
    }

    if parameter == RANDOMIZE_LAYERS as i32 {
        randomize_layers(context);
        return;
    }
    let layers = context.state().layers;
    let layer_field = parameter_layer(parameter, layers);
    if let Some((layer, LAYER_RANDOMIZE)) = layer_field {
        randomize_layer(context, layer);
        return;
    }
    if !(GAIN as i32..model_index(layers) as i32).contains(&parameter) {
        return;
    }
    #[cfg(feature = "profiling-no-click-rebuild")]
    if parameter == (model_index(0) + LAYER_FIRST_CONTROL + 6) as i32
        && context.state().models[0] == Model::PitchSweep
        && context.state().player.has_waveform()
    {
        return;
    }
    if let Some((layer, 0)) = layer_field {
        update_model_controls(context, layer, false);
    }
    context.state_mut().dirty = true;
}

fn process_render(context: &mut Callback<'_, Audio>, _frames: usize) {
    if let Some(slot) = context.state().in_flight {
        if let Ok((length, _timing)) =
            context.with_shared_resources(|work| (work.length, work.timing))
        {
            let state = context.state_mut();
            #[cfg(feature = "profiling")]
            state.meter.record_render(_timing);
            if let Some(length) = length {
                #[cfg(feature = "profiling-hold-playback")]
                let publish = !state.player.has_waveform();
                #[cfg(not(feature = "profiling-hold-playback"))]
                let publish = true;
                if publish {
                    #[cfg(feature = "profiling")]
                    let start = cpu_cycle_count();
                    state.player.publish(slot, length);
                    #[cfg(feature = "profiling")]
                    state
                        .meter
                        .record(profile::PUBLISH, cpu_cycle_count().wrapping_sub(start));
                }
            } else {
                state.dirty = true;
            }
            state.in_flight = None;
        }
    }
    if context.state().parameter_setup != ParameterSetup::Ready || !context.state().dirty {
        return;
    }
    if context.state().in_flight.is_some() {
        #[cfg(feature = "profiling")]
        context.state_mut().meter.wait_for_worker(_frames);
        return;
    }
    let state = context.state();
    let Some(slot) = state.player.free_buffer(usize::from(state.output_buffers)) else {
        #[cfg(feature = "profiling")]
        context.state_mut().meter.wait_for_buffer(_frames);
        return;
    };
    #[cfg(feature = "profiling")]
    let submit_started = cpu_cycle_count();
    let mut job = Job {
        rate: state.rate,
        output_buffers: state.output_buffers,
        ..Job::EMPTY
    };
    let parameters = context.read_parameters();
    job.gain = parameters.scaled(GAIN).unwrap_or(0.0);
    job.cutoff = cutoff_hz(parameters.value(CUTOFF).unwrap_or(CUTOFF_OFF));
    for (layer, (snapshot, &model)) in job
        .layers
        .iter_mut()
        .zip(&context.state().models)
        .enumerate()
        .take(context.state().layers)
    {
        let base = model_index(layer);
        snapshot.patch.model = model;
        snapshot.level = parameters
            .value(base + LAYER_LEVEL)
            .unwrap_or(0)
            .clamp(0, 100) as f32
            / 100.0;
        snapshot.solo = parameters.value(base + LAYER_SOLO) == Some(1);
        snapshot.mute = parameters.value(base + LAYER_MUTE) == Some(1);
        for (index, (decoded, control)) in snapshot
            .patch
            .controls
            .iter_mut()
            .zip(model.controls())
            .enumerate()
        {
            let value = parameters
                .value(base + LAYER_FIRST_CONTROL + index)
                .unwrap_or(control.initial);
            *decoded = control.decode(value);
        }
    }
    if context.with_shared_resources(|work| work.job = job).is_ok()
        && context.submit_audio_worker(slot as u32).is_ok()
    {
        context.state_mut().in_flight = Some(slot);
        context.state_mut().dirty = false;
    }
    #[cfg(feature = "profiling")]
    context.state_mut().meter.record(
        profile::SUBMIT,
        cpu_cycle_count().wrapping_sub(submit_started),
    );
}

fn write_text(output: &mut [c_char], text: &[u8]) -> i32 {
    let length = text.len();
    let Some(output) = output.get_mut(..=length) else {
        return 0;
    };
    for (destination, &byte) in output.iter_mut().zip(text) {
        *destination = byte as c_char;
    }
    output[length] = 0;
    length as i32
}

impl Plugin for KickLab {
    type State = State;
    type Region = ();
    type StateMemory = Sram;
    type ExtraRegions = Resources;

    const GUID: u32 = multichar(b'K', b'L', b'a', b'b');
    const NAME: &'static CStr = c"KickLab";
    const DESCRIPTION: &'static CStr = c"Layered kick drum prototype";
    const TAGS: u32 = TAG_INSTRUMENT;
    const PARAMETER_TEXT: bool = true;
    const AUDIO_WORKER_STACK_SIZE: u32 = 2048;
    const INITIALIZATION_IN_DRAM: bool = true;
    const STATIC_INITIALIZATION: bool = true;
    const SHARED_STATIC_DRAM: Option<&'static SharedStaticDram<Self>> = Some(&TABLE_DRAM);
    const SPECIFICATIONS: &'static [Specification] = &[
        Specification {
            name: c"Max Layers",
            min: 1,
            max: MAX_LAYERS as i32,
            default: 2,
            kind: TYPE_GENERIC,
        },
        Specification {
            name: c"Output Buffers",
            min: MIN_OUTPUT_BUFFERS as i32,
            max: MAX_OUTPUT_BUFFERS as i32,
            default: DEFAULT_OUTPUT_BUFFERS as i32,
            kind: TYPE_GENERIC,
        },
    ];

    fn configuration(specs: &[i32]) -> Option<Configuration> {
        let layers = layer_count(specs)?;
        output_buffer_count(specs)?;
        Some(Configuration {
            parameters: model_index(layers),
            pages: layers + 1,
            page_indices: model_index(layers),
            regions: 0,
        })
    }

    fn dram_buffer_len(_: &[i32], rate: u32) -> usize {
        if (32_000..=192_000).contains(&rate) {
            render::max_frames(rate)
        } else {
            0
        }
    }

    fn dram_buffer_count(specs: &[i32]) -> usize {
        match (output_buffer_count(specs), layer_count(specs)) {
            (Some(output_buffers), Some(layers)) => layer_slot(output_buffers, layers),
            _ => 0,
        }
    }

    fn dram_buffer_len_at(specs: &[i32], rate: u32, slot: usize) -> usize {
        let Some(output_buffers) = output_buffer_count(specs) else {
            return 0;
        };
        if slot >= output_buffers && (slot - output_buffers) % 2 == 1 {
            render::layer_cache_frames(rate)
        } else {
            Self::dram_buffer_len(specs, rate)
        }
    }

    fn initialize_state(
        destination: &mut RegionInitializer<'_, State>,
        specs: &[i32],
        rate: u32,
    ) -> Option<()> {
        if Self::dram_buffer_len(specs, rate) == 0 {
            return None;
        }
        distingnt_api::initialize_region!(
            destination,
            State {
                layers: layer_count(specs)?,
                rate: rate,
                parameter_setup: ParameterSetup::New,
                dirty: true,
                randomizing: 0,
                output_buffers: output_buffer_count(specs)? as u8,
                random_seed: distingnt_api::api::cpu_cycle_count() | 1,
                in_flight: None,
                meter: profile::Meter::EMPTY,
            },
            arrays {
                models: |_| Model::Off
            },
            regions {
                player: |player| Player::initialize(player, rate)
            }
        );
        Some(())
    }

    fn initialize_region(_: &mut RegionInitializer<'_, ()>, _: &[i32], _: usize) -> Option<()> {
        None
    }

    fn initialize_shared_resources(
        destination: &mut RegionInitializer<'_, RenderWork>,
        _: &[i32],
    ) -> Option<()> {
        distingnt_api::initialize_region!(
            destination,
            RenderWork {
                length: None,
                timing: profile::NO_RENDER_TIMING,
            },
            arrays {
                cached_layers: |_| render::CachedLayer::EMPTY
            },
            regions {
                job: Job::initialize
            }
        );
        Some(())
    }

    fn metadata(specs: &[i32], metadata: &mut Metadata<'_>) -> Option<()> {
        let routing = [
            definition(
                c"Trigger",
                0,
                LAST_BUS as i16,
                1,
                UNIT_CV_INPUT,
                SCALING_NONE,
            ),
            definition(
                c"Output",
                0,
                LAST_BUS as i16,
                13,
                UNIT_AUDIO_OUTPUT,
                SCALING_NONE,
            ),
            definition(c"Output Mode", 0, 1, 1, UNIT_OUTPUT_MODE, SCALING_NONE),
            definition(c"Master Gain", -360, 120, 0, UNIT_DB, SCALING_10),
            definition(
                c"Low-pass",
                0,
                CUTOFF_OFF,
                CUTOFF_OFF,
                UNIT_HAS_STRINGS,
                SCALING_NONE,
            ),
        ];
        for (index, item) in routing.into_iter().enumerate() {
            metadata.set_parameter(index, item)?;
        }
        metadata.set_parameter(
            RANDOMIZE_LAYERS,
            definition(c"Randomize Layers", 0, 1, 0, UNIT_CONFIRM, SCALING_NONE),
        )?;
        metadata.add_page(c"KickLab", 0, &[0, 1, 2, 3, 4, RANDOMIZE_LAYERS as u8])?;
        let layers = layer_count(specs)?;
        for (layer, &name) in LAYER_NAMES.iter().enumerate().take(layers) {
            let base = model_index(layer);
            let default_model = if layer == 0 { 1 } else { 0 };
            metadata.set_parameter(
                base,
                definition(
                    c"Model",
                    0,
                    (MODEL_NAMES.len() - 1) as i16,
                    default_model,
                    UNIT_CONFIRM,
                    SCALING_NONE,
                ),
            )?;
            metadata.set_parameter(base + LAYER_RANDOMIZE, RANDOMIZE)?;
            metadata.set_parameter(base + LAYER_LEVEL, LEVEL.definition()?)?;
            for (offset, name) in [(LAYER_SOLO, c"Solo"), (LAYER_MUTE, c"Mute")] {
                let toggle =
                    Definition::new(name, 0, 1, 0, UNIT_ENUM, SCALING_NONE, Some(OFF_ON.table()))?;
                metadata.set_parameter(base + offset, toggle)?;
            }
            let defaults = if layer == 0 {
                FIRST_LAYER_PITCH_SWEEP.as_slice()
            } else {
                &[]
            };
            for slot in 0..CONTROLS {
                let initial = defaults.get(slot).copied().unwrap_or(0);
                // Start with bounds that admit the stored raw values of any model.
                metadata.set_parameter(
                    base + LAYER_FIRST_CONTROL + slot,
                    definition(
                        c"Loading",
                        i16::MIN,
                        i16::MAX,
                        initial,
                        UNIT_NONE,
                        SCALING_NONE,
                    ),
                )?;
            }
            let indices = layer_page_indices(layer);
            metadata.add_page(name, 1, &indices)?;
        }
        Some(())
    }

    fn parameter_string(
        context: &mut Callback<'_, ReadOnly>,
        parameter: i32,
        value: i32,
        output: &mut [c_char],
    ) -> i32 {
        let layer_field = parameter_layer(parameter, context.state().layers);
        if parameter == RANDOMIZE_LAYERS as i32 || matches!(layer_field, Some((_, LAYER_RANDOMIZE)))
        {
            return write_text(output, if value == 0 { b"Ready" } else { b"Randomize?" });
        }
        if parameter == CUTOFF as i32 {
            let hz = cutoff_hz(value as i16);
            if hz == 0 {
                return write_text(output, b"Off");
            }
            if let Some(length) =
                distingnt_api::api::text::format_integer_chars(output, i32::from(hz))
            {
                return output.get_mut(length..).map_or(length as i32, |suffix| {
                    length as i32 + write_text(suffix, b" Hz")
                });
            }
        }
        if matches!(layer_field, Some((_, 0))) {
            if let Some(name) = MODEL_NAMES.get(value as usize) {
                return write_text(output, name.to_bytes());
            }
        }
        0
    }

    fn prefix(context: &mut Callback<'_, ReadOnly>, parameter: i32, output: &mut [c_char]) -> i32 {
        let layers = context.state().layers;
        parameter_layer(parameter, layers)
            .map(|(layer, _)| layer)
            .and_then(|layer| LAYER_PREFIXES.get(layer))
            .map_or(0, |prefix| write_text(output, prefix))
    }

    fn parameter_changed(context: &mut Callback<'_, Audio>, parameter: i32) {
        #[cfg(feature = "profiling")]
        let start = cpu_cycle_count();
        handle_parameter_change(context, parameter);
        #[cfg(feature = "profiling")]
        {
            let elapsed = cpu_cycle_count().wrapping_sub(start);
            context
                .state_mut()
                .meter
                .record(profile::PARAMETER_CHANGED, elapsed);
        }
    }

    fn calculate_static_requirements() -> StaticRequirements {
        StaticRequirements {
            dram: lookup::BYTES,
        }
    }

    fn initialise(memory: StaticMemory<'_>) {
        lookup::initialize(memory);
    }

    fn audio_worker_with_static_dram(
        work: &mut RenderWork,
        slot: u32,
        buffers: &WaveBuffers,
        memory: StaticDramView<'_>,
    ) {
        let tables = lookup::Lookup::from_dram(memory);
        let mut timing = profile::NO_RENDER_TIMING;
        work.length = buffers
            .with(slot as usize, |samples| {
                #[cfg(feature = "profiling")]
                let start = cpu_cycle_count();
                let length = render::render(
                    &work.job,
                    &mut work.cached_layers,
                    buffers,
                    samples,
                    tables,
                    &mut timing,
                );
                #[cfg(feature = "profiling")]
                {
                    timing.total = Some(cpu_cycle_count().wrapping_sub(start));
                }
                length
            })
            .flatten();
        work.timing = timing;
    }

    fn step(context: &mut Callback<'_, Audio>, buses: &mut [f32], frames: usize) {
        #[cfg(feature = "profiling")]
        let start = cpu_cycle_count();
        process_render(context, frames);
        #[cfg(feature = "profiling")]
        let scheduled = cpu_cycle_count();
        let parameters = context.read_parameters();
        let trigger = parameters.value(TRIGGER).and_then(bus_index);
        let output = parameters.value(OUTPUT).and_then(bus_index);
        let replace = parameters.value(OUTPUT_MODE) == Some(1);
        let mut player = context.state().player;
        let mut diagnostics = playback::PlaybackDiagnostics::default();
        player.play(
            context.dram_buffers(),
            buses,
            frames,
            trigger,
            output,
            replace,
            &mut diagnostics,
        );
        context.state_mut().player = player;
        #[cfg(feature = "profiling")]
        {
            let elapsed = cpu_cycle_count().wrapping_sub(start);
            // Read the bus after all KickLab voices have been summed. In Add
            // mode it can also contain earlier algorithms' contributions.
            let peak = output
                .and_then(|index| buses.get(index * frames..(index + 1) * frames))
                .map(|samples| {
                    samples
                        .iter()
                        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
                });
            let state = context.state_mut();
            state.meter.record(profile::STEP, elapsed);
            state
                .meter
                .record(profile::SCHEDULE, scheduled.wrapping_sub(start));
            state
                .meter
                .record_playback(start, diagnostics, peak, replace);
            state.meter.record_callback_entry(start);
            state.meter.advance(frames, state.rate);
            if let Some(packet) = state.meter.packet(state.rate) {
                let send_started = cpu_cycle_count();
                context.send_midi_sysex(DESTINATION_USB, packet.as_bytes());
                let send_elapsed = cpu_cycle_count().wrapping_sub(send_started);
                context
                    .state_mut()
                    .meter
                    .record(profile::TELEMETRY_SEND, send_elapsed);
            }
            let exit = cpu_cycle_count();
            context.state_mut().meter.record_callback_exit(start, exit);
        }
    }
}

distingnt_api::export_plugin!(KickLab);

#[cfg(test)]
mod tests;
