//! Layered mono kick drum with confirmed model selection and rendered hits.
#![no_std]
#![cfg_attr(not(test), forbid(unsafe_code))]

#[path = "../../layered-drums/layer_page.rs"]
mod layer_page;
#[path = "../../layered-drums/lookup.rs"]
mod lookup;
#[path = "../../layered-drums/playback.rs"]
mod playback;
#[cfg(feature = "profiling")]
mod profile;
#[path = "../../layered-drums/random.rs"]
mod random;
mod render;
#[path = "../../layered-drums/scope.rs"]
mod scope;
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

use distingnt_api::api::cpu_cycle_count;

use core::ffi::{c_char, CStr};
use distingnt_api::{
    api::{
        audio::bus_index,
        background::DramBuffers,
        drawing::Canvas,
        metadata::{Definition, EnumStrings, Name},
        parameters::{scaled_value, Audio, ReadOnly, Ui},
        plugin::{
            Configuration, Context, Metadata, Plugin, RegionInitializer, SharedResources,
            SharedStaticDram, Specification, StaticDramView, StaticMemory, StaticRequirements,
        },
        state_memory::Sram,
    },
    *,
};
use layer_page::{
    add_layer_controls, definition, page_name_storage, page_names, write_text, PageNameBytes,
    LAYER_FIRST_CONTROL, LAYER_LEVEL, LAYER_MUTE, LAYER_RANDOMIZE, LAYER_SOLO,
};
use layer_page::LAYER_LEVEL_RANGE;
use playback::Player;
use random::random_value;
use scope::Scope;

const MAX_LAYERS: usize = 8;
const MIN_OUTPUT_BUFFERS: usize = 2;
const DEFAULT_OUTPUT_BUFFERS: usize = 6;
const MAX_OUTPUT_BUFFERS: usize = 10;
const TRIGGER: usize = 0;
const OUTPUT: usize = 1;
const OUTPUT_MODE: usize = 2;
const CUTOFF: usize = 3;
const MASTER_DRIVE: usize = 4;
const GAIN: usize = 5;
const CUTOFF_OFF: i16 = 101;
const RANDOMIZE_LAYERS: usize = 6;
const GLOBALS: usize = 7;
const CONTROLS: usize = 10;
const LAYER_FIELDS: usize = LAYER_FIRST_CONTROL + CONTROLS;
const LAYER_PREFIXES: [&[u8]; MAX_LAYERS] = [
    b"L1: ", b"L2: ", b"L3: ", b"L4: ", b"L5: ", b"L6: ", b"L7: ", b"L8: ",
];
const MODEL_NAMES: [&CStr; 13] = [
    c"Off",
    c"Pitch Sweep",
    c"Modal Drum",
    c"Bit Logic",
    c"Grain Cloud",
    c"Hard Sync",
    c"Resonator",
    c"Two Op. FM",
    c"Noise Sweep",
    c"Ring Mod",
    c"Wavefold",
    c"Delay Drum",
    c"Spect. Pulse",
];
static LAYER_PAGE_NAME_STORAGE: [[PageNameBytes; MODEL_NAMES.len()]; MAX_LAYERS] =
    page_name_storage(&MODEL_NAMES);
static LAYER_PAGE_NAMES: [[Name; MODEL_NAMES.len()]; MAX_LAYERS] =
    page_names(&LAYER_PAGE_NAME_STORAGE);
static OPERATIONS: EnumStrings<4> = EnumStrings::new([c"XOR", c"AND", c"OR", c"Wrap Add"]);
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
    tune(c"Tail Tune", 900),
    decay(500),
    drive(164),
    depth(10),
    time(50),
    Control::new(c"Sweep Curve", -100, 100, 15, UNIT_PERCENT, SCALING_NONE),
    percent(c"Click Level", 20),
    Control::new(c"Click Tone", 1100, 10_000, 3000, UNIT_HZ, SCALING_NONE),
    Control::new(
        c"Click Decay",
        10,
        PITCH_CLICK_MAX_DECAY_TENTHS_MS,
        160,
        UNIT_MS,
        SCALING_10,
    ),
];
const MODAL_DRUM: [Control; 8] = [
    tune(c"Tail Tune", 1100),
    decay(400),
    drive(150),
    percent(c"Upper Modes", 60),
    percent(c"Damping", 20),
    percent(c"Strike Position", 25),
    percent(c"Beater Level", 50),
    percent(c"Beater Hardness", 25),
];
const BIT_LOGIC: [Control; 10] = [
    tune(c"Base Tune", 950),
    decay(750),
    drive(120),
    depth(19),
    time(100),
    Control::new(c"Ramp Ratio", 100, 600, 160, UNIT_NONE, SCALING_100),
    Control::new(c"Sample Clock", 500, 15_000, 1000, UNIT_HZ, SCALING_NONE),
    Control::new(c"Ramp Bits", 3, 12, 6, UNIT_NONE, SCALING_NONE),
    Control::new(c"Logic Operation", 0, 3, 1, UNIT_ENUM, SCALING_NONE),
    Control::new(c"Tone Cutoff", 160, 8500, 3300, UNIT_HZ, SCALING_NONE),
];
const GRAIN_SIZE_MAX_MS: i16 = 100;
const GRAIN_CLOUD: [Control; 10] = [
    tune(c"Tail Tune", 1100),
    decay(790),
    depth(18),
    time(123),
    Control::new(
        c"Grain Size",
        5,
        GRAIN_SIZE_MAX_MS,
        5,
        UNIT_MS,
        SCALING_NONE,
    ),
    Control::new(c"Grain Rate", 18, 220, 170, UNIT_HZ, SCALING_NONE),
    Control::new(c"Density Envelope", -30, 30, 13, UNIT_NONE, SCALING_10),
    Control::new(c"Pitch Spread", 0, 240, 60, UNIT_SEMITONES, SCALING_10),
    percent(c"Timing Jitter", 83),
    drive(100),
];
const HARD_SYNC: [Control; 8] = [
    tune(c"Tail Tune", 950),
    decay(725),
    depth(36),
    time(160),
    Control::new(c"Sync Bend", 0, 120, 17, UNIT_NONE, SCALING_10),
    Control::new(c"Sync Decay", 5, 500, 100, UNIT_MS, SCALING_NONE),
    Control::new(c"Tail Ratio", 100, 600, 220, UNIT_NONE, SCALING_100),
    drive(150),
];
const RESONATOR: [Control; 9] = [
    tune(c"Tail Tune", 375),
    decay(1000),
    depth(24),
    time(40),
    Control::new(c"Tail Pitch Drift", 0, 60, 35, UNIT_SEMITONES, SCALING_10),
    percent(c"Excitation", 50),
    Control::new(c"Exciter Width", 20, 300, 200, UNIT_MS, SCALING_100),
    Control::new(c"Tone Cutoff", 180, 8000, 5500, UNIT_HZ, SCALING_NONE),
    drive(150),
];
const TWO_OPERATOR_FM: [Control; 9] = [
    tune(c"Tail Tune", 870),
    decay(926),
    depth(42),
    time(32),
    Control::new(c"FM Amount", 0, 900, 162, UNIT_NONE, SCALING_100),
    Control::new(c"Modulator Ratio", 50, 1600, 1008, UNIT_NONE, SCALING_100),
    Control::new(c"FM Decay", 2, 400, 307, UNIT_MS, SCALING_NONE),
    Control::new(
        c"Modulator Sweep",
        -24,
        36,
        -23,
        UNIT_SEMITONES,
        SCALING_NONE,
    ),
    drive(65),
];
const NOISE_SWEEP: [Control; 8] = [
    tune(c"Tail Center", 941),
    decay(483),
    depth(46),
    time(125),
    Control::new(c"Filter Q", 5, 120, 87, UNIT_NONE, SCALING_10),
    Control::new(c"Q Sweep", -30, 30, -25, UNIT_NONE, SCALING_10),
    percent(c"Grain", 79),
    drive(150),
];
const RING_MOD: [Control; 9] = [
    tune(c"Tail Tune", 500),
    decay(750),
    depth(10),
    time(93),
    Control::new(c"Lower Osc Ratio", 50, 900, 485, UNIT_NONE, SCALING_100),
    percent(c"Clang Level", 90),
    Control::new(c"Clang Decay", 2, 600, 425, UNIT_MS, SCALING_NONE),
    Control::new(c"Clang Pitch Env", -24, 24, 3, UNIT_SEMITONES, SCALING_NONE),
    drive(180),
];
const WAVEFOLD: [Control; 9] = [
    tune(c"Tail Tune", 960),
    decay(730),
    depth(24),
    time(135),
    Control::new(c"Fold Amount", 0, 100, 80, UNIT_NONE, SCALING_10),
    Control::new(c"Fold Decay", 2, 650, 493, UNIT_MS, SCALING_NONE),
    Control::new(c"Fold Bias", -80, 80, 78, UNIT_PERCENT, SCALING_NONE),
    percent(c"Clean Sub Blend", 15),
    drive(105),
];
const DELAY_DRUM: [Control; 7] = [
    tune(c"Tail Tune", 700),
    decay(970),
    Control::new(c"Loop Cutoff", 250, 8500, 5200, UNIT_HZ, SCALING_NONE),
    Control::new(c"Strike Mix", 0, 200, 53, UNIT_PERCENT, SCALING_NONE),
    Control::new(c"Strike Width", 5, 200, 91, UNIT_MS, SCALING_10),
    Control::new(c"Strike Tone", 200, 8500, 3000, UNIT_HZ, SCALING_NONE),
    drive(125),
];
const SPECTRAL_PULSE: [Control; 7] = [
    tune(c"Tail Tune", 1040),
    Control::new(c"Body Decay", 70, 850, 735, UNIT_MS, SCALING_NONE),
    percent(c"Snap Level", 100),
    Control::new(c"Snap Center", 180, 3800, 1549, UNIT_HZ, SCALING_NONE),
    Control::new(c"Snap Bandwidth", 5, 100, 50, UNIT_PERCENT, SCALING_NONE),
    Control::new(c"Dispersion", 0, 200, 191, UNIT_MS, SCALING_10),
    drive(120),
];
const UNUSED: Definition = definition(c"Unused", 0, 0, 0, UNIT_NONE, SCALING_NONE);

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
    drive: f32,
    cutoff: i16,
    output_buffers: u8,
    layer_count: u8,
}
impl Job {
    const EMPTY: Self = Self {
        layers: [Layer::OFF; MAX_LAYERS],
        rate: 0,
        gain: 0.0,
        drive: 0.0,
        cutoff: 0,
        output_buffers: DEFAULT_OUTPUT_BUFFERS as u8,
        layer_count: 0,
    };

    fn initialize(destination: &mut RegionInitializer<'_, Self>) {
        distingnt_api::initialize_region!(
            destination,
            Job {
                rate: 0,
                gain: 0.0,
                drive: 0.0,
                cutoff: 0,
                output_buffers: DEFAULT_OUTPUT_BUFFERS as u8,
                layer_count: 0,
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
    grains: render::GrainWork,
    plan: Option<render::RenderPlan>,
    pending: bool,
    // The worker alone uses these keys and the per-layer DRAM slots.
    cached_layers: [render::CachedLayer; MAX_LAYERS],
    length: Option<usize>,
    timing: profile::RenderTiming,
    scope: Scope,
}

impl RenderWork {
    #[inline(never)]
    fn prepare_plan(&mut self) {
        // Plan construction can use its own frame before entering the renderer.
        self.plan = Some(render::RenderPlan::new(
            &self.job,
            render::max_frames(self.job.rate),
        ));
    }

    #[inline(never)]
    fn advance_render(
        &mut self,
        slot: u32,
        buffers: &WaveBuffers,
        memory: StaticDramView<'_>,
        start: u32,
    ) {
        let tables = lookup::Lookup::from_dram(memory);
        let status = buffers
            .with(slot as usize, |samples| {
                let plan = self.plan.as_mut()?;
                let mut render = |tile: &mut [f32]| {
                    plan.advance(
                        &self.job,
                        &mut self.cached_layers,
                        buffers,
                        samples,
                        tile,
                        tables,
                        &mut self.timing,
                        &mut self.scope,
                        start,
                    )
                };
                let layers = usize::from(self.job.layer_count);
                if layers >= MIN_TILED_LAYERS {
                    let tile_slot = layer_slot(usize::from(self.job.output_buffers), layers);
                    buffers.with(tile_slot, render).flatten()
                } else {
                    render(&mut [])
                }
            })
            .flatten();
        self.pending = matches!(status, Some(render::RenderStatus::Pending));
        match status {
            Some(render::RenderStatus::Complete(length)) => self.length = Some(length),
            None => {
                self.length = None;
                for layer in &mut self.cached_layers {
                    layer.discard_prepared_grains();
                }
            }
            Some(render::RenderStatus::Pending) => {}
        }
    }
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
    waiting_for_buffer: bool,
    randomizing: u16, // One bit per layer, followed by the global action.
    output_buffers: u8,
    random_seed: u32,
    in_flight: Option<usize>,
    buffer_usage: playback::BufferUsage,
    player: Player,
    scope: Scope,
    #[cfg_attr(not(feature = "profiling"), allow(dead_code))]
    meter: profile::Meter,
}

struct KickLab;
static TABLE_DRAM: SharedStaticDram<KickLab> = SharedStaticDram::new();
const MIX_TILE_FRAMES: usize = 512;
const MIN_TILED_LAYERS: usize = 3;
type WaveBuffers = DramBuffers<f32, { MAX_OUTPUT_BUFFERS + 2 * MAX_LAYERS + 1 }>;
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
    if let (Some(names), Some(indices)) = (LAYER_PAGE_NAMES.get(layer), indices.get(..visible)) {
        context.set_named_page(layer + 1, names[model as usize], indices);
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
    let layers = context.state().layers;
    let mut next_active = 0;
    let mut off_start = layers;
    for _ in 0..layers {
        let model_value = random_value(
            &mut context.state_mut().random_seed,
            0,
            (MODEL_NAMES.len() - 1) as i16,
        );
        let layer = if model_value == 0 {
            off_start -= 1;
            off_start
        } else {
            let layer = next_active;
            next_active += 1;
            layer
        };
        let base = model_index(layer);
        if context.read_parameters().value(base) != Some(model_value) {
            context.parameters().set(base, model_value);
            // Refresh definitions even when the host defers its notification.
            update_model_controls(context, layer, false);
        }
        let [min, max] = LAYER_LEVEL_RANGE;
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
    if !(CUTOFF as i32..model_index(layers) as i32).contains(&parameter) {
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
        let resume = context.with_shared_state(|state, work| {
            if work.pending {
                return true;
            }
            #[cfg(feature = "profiling")]
            state.meter.record_render(work.timing);
            if let Some(length) = work.length {
                #[cfg(feature = "profiling-hold-playback")]
                let publish = !state.player.has_waveform();
                #[cfg(not(feature = "profiling-hold-playback"))]
                let publish = true;
                if publish {
                    #[cfg(feature = "profiling")]
                    let start = cpu_cycle_count();
                    state.player.publish(slot, length);
                    state.scope = work.scope;
                    #[cfg(feature = "profiling")]
                    state
                        .meter
                        .record(profile::PUBLISH, cpu_cycle_count().wrapping_sub(start));
                }
            } else {
                state.dirty = true;
            }
            state.in_flight = None;
            false
        });
        if resume == Ok(true) {
            // The previous slice released the worker reservation. A full
            // firmware queue leaves this slot in-flight for the next retry.
            let _ = context.submit_audio_worker(slot as u32);
            return;
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
    let state = context.state_mut();
    let Some(slot) = state.player.free_buffer(usize::from(state.output_buffers)) else {
        if !state.waiting_for_buffer {
            state.waiting_for_buffer = true;
            state.buffer_usage.record_wait();
        }
        #[cfg(feature = "profiling")]
        state.meter.wait_for_buffer(_frames);
        return;
    };
    state.waiting_for_buffer = false;
    #[cfg(feature = "profiling")]
    let submit_started = cpu_cycle_count();
    let mut job = Job {
        rate: state.rate,
        output_buffers: state.output_buffers,
        layer_count: state.layers as u8,
        ..Job::EMPTY
    };
    let parameters = context.read_parameters();
    job.gain = parameters.scaled(GAIN).unwrap_or(0.0);
    job.drive = parameters.scaled(MASTER_DRIVE).unwrap_or(0.0);
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
    if context
        .with_shared_resources(|work| {
            work.job = job;
            work.grains = render::GrainWork::EMPTY;
            work.plan = None;
            work.pending = false;
            work.timing = profile::NO_RENDER_TIMING;
        })
        .is_ok()
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

impl Plugin for KickLab {
    type State = State;
    type Region = ();
    type StateMemory = Sram;
    type ExtraRegions = Resources;

    const GUID: u32 = multichar(b'A', b'T', b'k', b'l');
    const NAME: &'static CStr = c"KickLab";
    const DESCRIPTION: &'static CStr = c"Layered kick drum prototype";
    const TAGS: u32 = TAG_INSTRUMENT;
    const PARAMETER_TEXT: bool = true;
    const AUDIO_WORKER_STACK_SIZE: u32 = 2048;
    const INITIALIZATION_IN_DRAM: bool = true;
    const DRAWING_IN_DRAM: bool = true;
    const STATIC_INITIALIZATION: bool = true;
    const SHARED_STATIC_DRAM: Option<&'static SharedStaticDram<Self>> = Some(&TABLE_DRAM);
    const SPECIFICATIONS: &'static [Specification] = &[
        Specification {
            name: c"Max Layers",
            min: 1,
            max: MAX_LAYERS as i32,
            default: 1,
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
            (Some(output_buffers), Some(layers)) => {
                layer_slot(output_buffers, layers) + usize::from(layers >= MIN_TILED_LAYERS)
            }
            _ => 0,
        }
    }

    fn dram_buffer_len_at(specs: &[i32], rate: u32, slot: usize) -> usize {
        let Some(output_buffers) = output_buffer_count(specs) else {
            return 0;
        };
        if layer_count(specs).is_some_and(|layers| {
            layers >= MIN_TILED_LAYERS && slot == layer_slot(output_buffers, layers)
        }) {
            return MIX_TILE_FRAMES;
        }
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
                waiting_for_buffer: false,
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
                player: |player| Player::initialize(player, rate),
                scope: Scope::initialize,
                buffer_usage: playback::BufferUsage::initialize
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
                grains: render::GrainWork::EMPTY,
                plan: None,
                pending: false,
                timing: profile::NO_RENDER_TIMING,
            },
            arrays {
                cached_layers: |_| render::CachedLayer::EMPTY
            },
            regions {
                job: Job::initialize,
                scope: Scope::initialize
            }
        );
        Some(())
    }

    fn metadata(specs: &[i32], metadata: &mut Metadata<'_>) -> Option<()> {
        let globals = [
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
            definition(
                c"Low-pass",
                0,
                CUTOFF_OFF,
                CUTOFF_OFF,
                UNIT_HAS_STRINGS,
                SCALING_NONE,
            ),
            definition(c"Master Drive", 0, 240, 0, UNIT_DB, SCALING_10),
            definition(c"Master Gain", -360, 120, 0, UNIT_DB, SCALING_10),
            definition(c"Randomize Layers", 0, 1, 0, UNIT_CONFIRM, SCALING_NONE),
        ];
        for (index, item) in globals.into_iter().enumerate() {
            metadata.set_parameter(index, item)?;
        }
        let indices: [u8; GLOBALS] = core::array::from_fn(|index| index as u8);
        metadata.add_page(c"KickLab", 0, &indices)?;
        let layers = layer_count(specs)?;
        for (layer, names) in LAYER_PAGE_NAMES.iter().enumerate().take(layers) {
            let base = model_index(layer);
            let default_model = if layer == 0 { 1 } else { 0 };
            let model = Model::from_value(default_model);
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
            add_layer_controls(metadata, base)?;
            let controls = model.controls();
            for slot in 0..CONTROLS {
                let initial = controls.get(slot).map_or(0, |control| control.initial);
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
            metadata.add_named_page(names[model as usize], 1, &indices)?;
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

    distingnt_api::nt_dram_section! {
    fn audio_worker_with_static_dram(
        work: &mut RenderWork,
        slot: u32,
        buffers: &WaveBuffers,
        memory: StaticDramView<'_>,
    ) {
        let start = cpu_cycle_count();
        if work.plan.is_none() {
            work.pending = work.grains.advance(
                &work.job,
                &mut work.cached_layers,
                buffers,
                &mut work.timing,
                start,
            );
            if !work.pending {
                work.prepare_plan();
            }
        }
        if work.plan.is_some() {
            work.advance_render(slot, buffers, memory, start);
        }
        #[cfg(feature = "profiling")]
        {
            let elapsed = cpu_cycle_count().wrapping_sub(start);
            work.timing.total = Some(work.timing.total.unwrap_or(0).saturating_add(elapsed));
        }
    }
    }

    fn draw(context: &mut Callback<'_, Ui>, canvas: Canvas<'_>) -> bool {
        let state = context.state();
        state.scope.draw(
            canvas,
            state.buffer_usage.peak(),
            state.buffer_usage.waits(),
            c"1000 ms",
        );
        false
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
        let mut occupied = player.occupied_buffers();
        if let Some(slot) = context.state().in_flight {
            occupied |= 1 << slot;
        }
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
        let state = context.state_mut();
        state.player = player;
        // A trigger only reuses the published waveform. Playback can release
        // slots within this block, so measure occupancy before playing it.
        state
            .buffer_usage
            .observe(occupied.count_ones() as usize, frames, state.rate);
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
