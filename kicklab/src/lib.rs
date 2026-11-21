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
const BUFFERS: usize = 6;
const TRIGGER: usize = 0;
const OUTPUT: usize = 1;
const OUTPUT_MODE: usize = 2;
const GAIN: usize = 3;
const CUTOFF: usize = 4;
const CUTOFF_OFF: i16 = 101;
const GLOBALS: usize = 5;
const CONTROLS: usize = 10;
const LAYER_FIELDS: usize = 2 + CONTROLS;
// Keep all pre-existing ordinals, including the space used by larger presets.
const RANDOMIZE_START: usize = GLOBALS + MAX_LAYERS * LAYER_FIELDS;
const MIX_START: usize = RANDOMIZE_START + MAX_LAYERS;
const LAYER_PAGE_FIELDS: usize = LAYER_FIELDS + 3;
const LAYER_NAMES: [&CStr; MAX_LAYERS] = [c"Layer 1", c"Layer 2", c"Layer 3", c"Layer 4"];
const LAYER_PREFIXES: [&[u8]; MAX_LAYERS] = [b"L1: ", b"L2: ", b"L3: ", b"L4: "];
const MODEL_NAMES: [&CStr; 4] = [c"Off", c"Pitch Sweep", c"Modal Drum", c"Bit Logic"];
static OPERATIONS: EnumStrings<4> = EnumStrings::new([c"XOR", c"AND", c"OR", c"Wrap Add"]);
static OFF_ON: EnumStrings<2> = EnumStrings::new([c"Off", c"On"]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Model {
    Off,
    PitchSweep,
    ModalDrum,
    BitLogic,
}

impl Model {
    fn from_value(value: i16) -> Self {
        match value {
            1 => Self::PitchSweep,
            2 => Self::ModalDrum,
            3 => Self::BitLogic,
            _ => Self::Off,
        }
    }

    fn controls(self) -> &'static [Control] {
        match self {
            Self::Off => &[],
            Self::PitchSweep => &PITCH_SWEEP,
            Self::ModalDrum => &MODAL_DRUM,
            Self::BitLogic => &BIT_LOGIC,
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
}
impl Job {
    const EMPTY: Self = Self {
        layers: [Layer::OFF; MAX_LAYERS],
        rate: 0,
        gain: 0.0,
        cutoff: 0,
    };

    fn initialize(destination: &mut RegionInitializer<'_, Self>) {
        distingnt_api::initialize_region!(
            destination,
            Job {
                rate: 0,
                gain: 0.0,
                cutoff: 0
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
    random_seed: u32,
    in_flight: Option<usize>,
    player: Player,
    #[cfg_attr(not(feature = "profiling"), allow(dead_code))]
    meter: profile::Meter,
}

struct KickLab;
static TABLE_DRAM: SharedStaticDram<KickLab> = SharedStaticDram::new();
type WaveBuffers = DramBuffers<f32, { BUFFERS + 2 * MAX_LAYERS }>;
type Resources = SharedResources<RenderWork, (), WaveBuffers>;
type Callback<'a, Mode> = Context<'a, State, (), Mode, Sram, Resources>;

fn layer_slot(layer: usize) -> usize {
    BUFFERS + 2 * layer
}

fn layer_count(specs: &[i32]) -> Option<usize> {
    let count = usize::try_from(*specs.first()?).ok()?;
    (1..=MAX_LAYERS).contains(&count).then_some(count)
}

fn model_index(layer: usize) -> usize {
    GLOBALS + LAYER_FIELDS * layer
}

fn randomize_layer_index(parameter: i32, layers: usize) -> Option<usize> {
    let layer = usize::try_from(parameter)
        .ok()?
        .checked_sub(RANDOMIZE_START)?;
    (layer < layers).then_some(layer)
}

fn solo_index(layer: usize) -> usize {
    MIX_START + 2 * layer
}

fn mix_parameter_layer(parameter: i32, layers: usize) -> Option<usize> {
    let layer = usize::try_from(parameter).ok()?.checked_sub(MIX_START)? / 2;
    (layer < layers).then_some(layer)
}

fn layer_page_indices(layer: usize, controls: usize) -> [u8; LAYER_PAGE_FIELDS] {
    let base = model_index(layer);
    core::array::from_fn(|slot| {
        let index = match slot {
            0 | 1 => base + slot,
            2 | 3 => solo_index(layer) + slot - 2,
            _ if slot == 4 + controls => RANDOMIZE_START + layer,
            _ => base + slot - 2,
        };
        index as u8
    })
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
        let index = base + 2 + slot;
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
    let indices = layer_page_indices(layer, controls.len());
    let visible = if model == Model::Off {
        1
    } else {
        5 + controls.len()
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
        let action = RANDOMIZE_START + layer;
        if context.read_parameters().value(action) == Some(1) {
            context.parameters().set(action, 0);
        }
    }
    let state = context.state_mut();
    state.parameter_setup = ParameterSetup::Ready;
    state.dirty = true;
}

fn randomize_layer(context: &mut Callback<'_, Audio>, layer: usize) {
    let action = RANDOMIZE_START + layer;
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
        let base = model_index(layer);
        for (index, control) in model.controls().iter().enumerate() {
            let seed = &mut context.state_mut().random_seed;
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            let [min, max] = control.range;
            let width = (i32::from(max) - i32::from(min) + 1) as u32;
            let offset = ((u64::from(*seed) * u64::from(width)) >> 32) as i32;
            let value = i32::from(min) + offset;
            context.parameters().set(base + 2 + index, value as i16);
        }
        context.state_mut().dirty = true;
    }
    // Keep the same-layer guard set until the host's reset notification returns.
    context.parameters().set(action, 0);
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

    let layers = context.state().layers;
    if let Some(layer) = randomize_layer_index(parameter, layers) {
        randomize_layer(context, layer);
        return;
    }
    if !(GAIN as i32..model_index(layers) as i32).contains(&parameter)
        && mix_parameter_layer(parameter, layers).is_none()
    {
        return;
    }
    #[cfg(feature = "profiling-no-click-rebuild")]
    if parameter == (model_index(0) + 2 + 6) as i32
        && context.state().models[0] == Model::PitchSweep
        && context.state().player.has_waveform()
    {
        return;
    }
    if let Some((layer, 0)) = parameter_layer(parameter, layers) {
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
    let Some(slot) = context.state().player.free_buffer() else {
        #[cfg(feature = "profiling")]
        context.state_mut().meter.wait_for_buffer(_frames);
        return;
    };
    #[cfg(feature = "profiling")]
    let submit_started = cpu_cycle_count();
    let mut job = Job {
        rate: context.state().rate,
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
        snapshot.level = parameters.value(base + 1).unwrap_or(0).clamp(0, 100) as f32 / 100.0;
        let solo = solo_index(layer);
        snapshot.solo = parameters.value(solo) == Some(1);
        snapshot.mute = parameters.value(solo + 1) == Some(1);
        for (index, (decoded, control)) in snapshot
            .patch
            .controls
            .iter_mut()
            .zip(model.controls())
            .enumerate()
        {
            let value = parameters
                .value(base + 2 + index)
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
    const SPECIFICATIONS: &'static [Specification] = &[Specification {
        name: c"Max Layers",
        min: 1,
        max: MAX_LAYERS as i32,
        default: 2,
        kind: TYPE_GENERIC,
    }];

    fn configuration(specs: &[i32]) -> Option<Configuration> {
        let layers = layer_count(specs)?;
        Some(Configuration {
            parameters: solo_index(layers),
            pages: layers + 1,
            page_indices: GLOBALS + layers * LAYER_PAGE_FIELDS,
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
        layer_count(specs).map_or(0, layer_slot)
    }

    fn dram_buffer_len_at(specs: &[i32], rate: u32, slot: usize) -> usize {
        if slot >= BUFFERS && (slot - BUFFERS) % 2 == 1 {
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
        metadata.add_page(c"KickLab", 0, &[0, 1, 2, 3, 4])?;
        let layers = layer_count(specs)?;
        for index in model_index(layers)..RANDOMIZE_START {
            metadata.set_parameter(index, UNUSED)?;
        }
        for index in (RANDOMIZE_START + layers)..MIX_START {
            metadata.set_parameter(index, UNUSED)?;
        }
        for (layer, &name) in LAYER_NAMES.iter().enumerate().take(layers) {
            let base = model_index(layer);
            let default_model = if layer == 0 { 1 } else { 0 };
            metadata.set_parameter(
                base,
                definition(c"Model", 0, 3, default_model, UNIT_CONFIRM, SCALING_NONE),
            )?;
            metadata.set_parameter(base + 1, LEVEL.definition()?)?;
            let controls = Model::from_value(default_model).controls();
            for slot in 0..CONTROLS {
                let initial = controls.get(slot).map_or(0, |control| control.initial);
                // Start with bounds that admit the stored raw values of any model.
                metadata.set_parameter(
                    base + 2 + slot,
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
            metadata.set_parameter(RANDOMIZE_START + layer, RANDOMIZE)?;
            let solo = solo_index(layer);
            for (index, name) in [(solo, c"Solo"), (solo + 1, c"Mute")] {
                let toggle =
                    Definition::new(name, 0, 1, 0, UNIT_ENUM, SCALING_NONE, Some(OFF_ON.table()))?;
                metadata.set_parameter(index, toggle)?;
            }
            let indices = layer_page_indices(layer, CONTROLS);
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
        if randomize_layer_index(parameter, context.state().layers).is_some() {
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
        if matches!(
            parameter_layer(parameter, context.state().layers),
            Some((_, 0))
        ) {
            if let Some(name) = MODEL_NAMES.get(value as usize) {
                return write_text(output, name.to_bytes());
            }
        }
        0
    }

    fn prefix(context: &mut Callback<'_, ReadOnly>, parameter: i32, output: &mut [c_char]) -> i32 {
        let layers = context.state().layers;
        randomize_layer_index(parameter, layers)
            .or_else(|| mix_parameter_layer(parameter, layers))
            .or_else(|| parameter_layer(parameter, layers).map(|(layer, _)| layer))
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
