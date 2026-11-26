//! Layered snare synthesis with background-rendered hits.
#![no_std]
#![cfg_attr(not(test), forbid(unsafe_code))]

#[path = "../../layered-drums/layer.rs"]
mod layer;
#[path = "../../layered-drums/layer_page.rs"]
mod layer_page;
#[path = "../../layered-drums/lookup.rs"]
mod lookup;
#[path = "../../layered-drums/playback.rs"]
mod playback;
#[path = "../../layered-drums/random.rs"]
mod random;
mod render;
#[path = "../../layered-drums/scope.rs"]
mod scope;

use distingnt_api::api::cpu_cycle_count;

use core::ffi::{c_char, CStr};
use distingnt_api::{
    api::{
        audio::bus_index,
        background::DramBuffers,
        drawing::Canvas,
        metadata::{Definition, Name},
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
use playback::Player;
use random::random_value;
use scope::Scope;

const MIN_LAYERS: usize = 2;
const MAX_LAYERS: usize = 5;
const MIN_OUTPUT_BUFFERS: usize = 2;
const DEFAULT_OUTPUT_BUFFERS: usize = 6;
const MAX_OUTPUT_BUFFERS: usize = 10;
const TRIGGER: usize = 0;
const OUTPUT: usize = 1;
const OUTPUT_MODE: usize = 2;
const MASTER_DRIVE: usize = 3;
const GAIN: usize = 4;
const RANDOMIZE_LAYERS: usize = 5;
const GLOBALS: usize = 6;
const LEGACY_CONTROLS: usize = 9;
const CONTROLS: usize = 10;
const MAX_PAGE_FIELDS: usize = LAYER_FIRST_CONTROL + CONTROLS;
const LAYER_FIELDS: usize = LAYER_FIRST_CONTROL + LEGACY_CONTROLS;
const LAYER_PREFIXES: [&[u8]; MAX_LAYERS] = [b"L1: ", b"L2: ", b"L3: ", b"L4: ", b"L5: "];
const MODEL_NAMES: [&CStr; 9] = [
    c"Off",
    c"Pitch Sweep",
    c"Resonator",
    c"Modal Body",
    c"Two-Stage",
    c"Snare Wires",
    c"Reson. Wires",
    c"Sweep Wires",
    c"Rim Shot",
];
static LAYER_PAGE_NAME_STORAGE: [[PageNameBytes; MODEL_NAMES.len()]; MAX_LAYERS] =
    page_name_storage(&MODEL_NAMES);
static LAYER_PAGE_NAMES: [[Name; MODEL_NAMES.len()]; MAX_LAYERS] =
    page_names(&LAYER_PAGE_NAME_STORAGE);
#[derive(Clone, Copy, PartialEq, Eq)]
enum Model {
    Off,
    PitchSweep,
    Resonator,
    Modal,
    TwoStage,
    Wires,
    ResonantWires,
    SweepWires,
    Rim,
}

impl layer::LayerModel for Model {
    const OFF: Self = Self::Off;
}

impl Model {
    fn from_value(value: i16) -> Self {
        match value {
            1 => Self::PitchSweep,
            2 => Self::Resonator,
            3 => Self::Modal,
            4 => Self::TwoStage,
            5 => Self::Wires,
            6 => Self::ResonantWires,
            7 => Self::SweepWires,
            8 => Self::Rim,
            _ => Self::Off,
        }
    }

    fn controls(self) -> &'static [Control] {
        match self {
            Self::Off => &[],
            Self::PitchSweep => &PITCH_SWEEP,
            Self::Resonator => &RESONATOR,
            Self::Modal => &MODAL,
            Self::Wires => &WIRES,
            Self::SweepWires => &SWEEP_WIRES,
            Self::Rim => &RIM,
            Self::TwoStage => &TWO_STAGE,
            Self::ResonantWires => &RESONANT_WIRES,
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
    step: i16,
}

impl Control {
    const fn new(
        name: &'static CStr,
        min: i16,
        max: i16,
        initial: i16,
        unit: u8,
        scale: u8,
        step: i16,
    ) -> Self {
        Self {
            name,
            range: [min, max],
            initial,
            unit,
            scale,
            step,
        }
    }

    fn definition(self) -> Option<Definition> {
        Definition::new(
            self.name,
            self.range[0],
            self.range[1],
            self.initial,
            self.unit,
            self.scale,
            None,
        )
    }

    fn decode(self, value: i16) -> f32 {
        scaled_value(value.max(self.range[0]).min(self.range[1]), self.scale)
    }
}

const fn hz(name: &'static CStr, min: i16, max: i16, initial: i16, step: i16) -> Control {
    Control::new(name, min, max, initial, UNIT_HZ, SCALING_NONE, step)
}
const fn ms(name: &'static CStr, min: i16, max: i16, initial: i16) -> Control {
    Control::new(name, min, max, initial, UNIT_MS, SCALING_NONE, 1)
}
const fn percent(name: &'static CStr, initial: i16) -> Control {
    Control::new(name, 0, 100, initial, UNIT_PERCENT, SCALING_NONE, 1)
}
const fn drive(initial: i16) -> Control {
    Control::new(c"Drive", 0, 180, initial, UNIT_DB, SCALING_10, 1)
}
const fn head(initial: i16) -> Control {
    hz(c"Head Pitch", 110, 430, initial, 1)
}
const fn decay(initial: i16) -> Control {
    ms(c"Body Decay", 25, 500, initial)
}
const fn depth(initial: i16) -> Control {
    Control::new(
        c"Sweep Depth",
        0,
        12,
        initial,
        UNIT_SEMITONES,
        SCALING_NONE,
        1,
    )
}
const fn sweep_time(initial: i16) -> Control {
    ms(c"Sweep Time", 1, 85, initial)
}
const fn wire_tone(initial: i16) -> Control {
    hz(c"Wire Brightness", 800, 7800, initial, 10)
}
const fn wire_decay(initial: i16) -> Control {
    ms(c"Wire Decay", 20, 500, initial)
}
const WIRE_DELAY: Control = Control::new(c"Wire Delay", 0, 240, 30, UNIT_MS, SCALING_10, 1);

const BODY_HOLD: Control = ms(c"Body Hold", 0, 40, 0);
const WIRE_ATTACK: Control = Control::new(c"Wire Attack", 1, 350, 7, UNIT_MS, SCALING_10, 1);
const PITCH_SWEEP: [Control; 10] = [
    head(195),
    decay(110),
    depth(5),
    sweep_time(18),
    Control::new(c"Sweep Curve", -100, 100, 0, UNIT_PERCENT, SCALING_NONE, 1),
    percent(c"Click Level", 0),
    hz(c"Click Tone", 1100, 10000, 5500, 10),
    Control::new(c"Click Decay", 10, 450, 60, UNIT_MS, SCALING_10, 1),
    drive(10),
    BODY_HOLD,
];
const RESONATOR: [Control; 10] = [
    head(205),
    decay(155),
    depth(3),
    sweep_time(12),
    Control::new(c"Tail Pitch Drift", 0, 60, 6, UNIT_SEMITONES, SCALING_10, 1),
    percent(c"Excitation", 65),
    Control::new(c"Exciter Width", 20, 150, 50, UNIT_MS, SCALING_100, 5),
    hz(c"Tone Cutoff", 180, 8000, 2800, 10),
    drive(10),
    BODY_HOLD,
];
const MODAL: [Control; 7] = [
    head(185),
    decay(155),
    percent(c"Upper Modes", 62),
    percent(c"Damping", 48),
    percent(c"Strike Position", 50),
    drive(10),
    BODY_HOLD,
];
const WIRES: [Control; 5] = [
    wire_tone(3400),
    wire_decay(260),
    WIRE_DELAY,
    drive(0),
    WIRE_ATTACK,
];
const SWEEP_WIRES: [Control; 7] = [
    wire_tone(3200),
    wire_decay(250),
    WIRE_DELAY,
    Control::new(c"Filter Q", 50, 250, 95, UNIT_NONE, SCALING_100, 5),
    percent(c"Grain", 84),
    drive(0),
    WIRE_ATTACK,
];
const RIM: [Control; 10] = [
    hz(c"Rim Pitch", 400, 3000, 979, 1),
    ms(c"Rim Decay", 8, 130, 16),
    hz(c"Click Brightness", 2600, 14000, 5900, 10),
    ms(c"Click Decay", 3, 60, 37),
    drive(0),
    percent(c"Rim Contour", 100),
    Control::new(c"Pitch Bend", 0, 150, 100, UNIT_PERCENT, SCALING_NONE, 1),
    Control::new(c"Snap Level", 0, 150, 100, UNIT_PERCENT, SCALING_NONE, 1),
    percent(c"Click Focus", 100),
    Control::new(c"Click Level", 0, 150, 100, UNIT_PERCENT, SCALING_NONE, 1),
];
const TWO_STAGE: [Control; 5] = [
    head(174),
    hz(c"Strike Pitch", 280, 900, 485, 1),
    Control::new(c"Handoff Time", 10, 300, 107, UNIT_MS, SCALING_10, 1),
    Control::new(c"Body Hold", 0, 400, 167, UNIT_MS, SCALING_10, 1),
    Control::new(c"Body Length", 25, 260, 100, UNIT_PERCENT, SCALING_NONE, 1),
];
const RESONANT_WIRES: [Control; 7] = [
    Control::new(c"Wire Delay", 0, 600, 186, UNIT_MS, SCALING_10, 1),
    Control::new(c"Wire Rise", 10, 800, 311, UNIT_MS, SCALING_10, 1),
    Control::new(c"Wire Length", 25, 200, 100, UNIT_PERCENT, SCALING_NONE, 1),
    Control::new(
        c"Wire Brightness",
        0,
        200,
        100,
        UNIT_PERCENT,
        SCALING_NONE,
        1,
    ),
    hz(c"Tail Ring Pitch", 1000, 3000, 1674, 1),
    percent(c"Tail Ring Level", 100),
    Control::new(c"Noise Seed", 0, 99, 0, UNIT_NONE, SCALING_NONE, 1),
];

const UNUSED: Definition = definition(c"Unused", 0, 0, 0, UNIT_NONE, SCALING_NONE);

type LayerPatch = layer::LayerPatch<Model, CONTROLS>;
type Layer = layer::Layer<Model, CONTROLS>;

#[derive(Clone, Copy)]
struct Job {
    layers: [Layer; MAX_LAYERS],
    rate: u32,
    gain: f32,
    drive: f32,
    output_buffers: u8,
    layer_count: u8,
}
impl Job {
    const EMPTY: Self = Self {
        layers: [Layer::OFF; MAX_LAYERS],
        rate: 0,
        gain: 0.0,
        drive: 0.0,
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
    plan: Option<render::RenderPlan>,
    pending: bool,
    // The worker alone uses these keys and the per-layer DRAM slots.
    cached_layers: [render::CachedLayer; MAX_LAYERS],
    length: Option<usize>,
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
                let tile_slot = layer_slot(
                    usize::from(self.job.output_buffers),
                    usize::from(self.job.layer_count),
                );
                buffers
                    .with(tile_slot, |tile| {
                        buffers
                            .with(tile_slot + 1, |punch_tile| {
                                plan.advance(
                                    &self.job,
                                    &mut self.cached_layers,
                                    buffers,
                                    samples,
                                    tile,
                                    punch_tile,
                                    tables,
                                    &mut self.scope,
                                    start,
                                )
                            })
                            .flatten()
                    })
                    .flatten()
            })
            .flatten();
        self.pending = matches!(status, Some(render::RenderStatus::Pending));
        match status {
            Some(render::RenderStatus::Complete(length)) => self.length = Some(length),
            None => {
                self.length = None;
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
}

struct SnareLab;
static TABLE_DRAM: SharedStaticDram<SnareLab> = SharedStaticDram::new();
const MIX_TILE_FRAMES: usize = 512;
type WaveBuffers = DramBuffers<f32, { MAX_OUTPUT_BUFFERS + MAX_LAYERS + 2 }>;
type Resources = SharedResources<RenderWork, (), WaveBuffers>;
type Callback<'a, Mode> = Context<'a, State, (), Mode, Sram, Resources>;

fn layer_slot(output_buffers: usize, layer: usize) -> usize {
    output_buffers + layer
}

fn layer_count(specs: &[i32]) -> Option<usize> {
    let count = usize::try_from(*specs.first()?).ok()?;
    (MIN_LAYERS..=MAX_LAYERS).contains(&count).then_some(count)
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

// Reserve the legacy blocks at their original addresses. The tenth control
// lives in a fixed bank so neither old mappings nor respecification move it.
fn extra_control_index(layer: usize) -> usize {
    model_index(MAX_LAYERS) + layer
}

fn control_index(layer: usize, slot: usize) -> usize {
    if slot < LEGACY_CONTROLS {
        model_index(layer) + LAYER_FIRST_CONTROL + slot
    } else {
        extra_control_index(layer)
    }
}

fn layer_page_indices(layer: usize) -> [u8; MAX_PAGE_FIELDS] {
    let base = model_index(layer);
    core::array::from_fn(|slot| {
        if slot < LAYER_FIELDS {
            (base + slot) as u8
        } else {
            extra_control_index(layer) as u8
        }
    })
}

fn parameter_layer(parameter: i32, layers: usize) -> Option<(usize, usize)> {
    let parameter = usize::try_from(parameter).ok()?;
    if let Some(layer) = parameter.checked_sub(extra_control_index(0)) {
        return (layer < layers).then_some((layer, LAYER_FIELDS));
    }
    let offset = parameter.checked_sub(GLOBALS)?;
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
        let index = control_index(layer, slot);
        let control = controls.get(slot).copied();
        let def = control.and_then(Control::definition).unwrap_or(UNUSED);
        // Read saved controls before the host can clamp them to the new definition.
        let value = control.map_or(0, |control| {
            if restoring {
                let [min, max] = control.range;
                let stored = context.read_parameters().value(index).unwrap_or(i16::MIN);
                let legacy_wire_attack = (model == Model::Wires && slot == 4)
                    || (model == Model::SweepWires && slot == 6);
                if stored == i16::MIN || (legacy_wire_attack && stored == 0) {
                    control.initial
                } else {
                    stored.max(min).min(max)
                }
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

fn randomize_model_controls(context: &mut Callback<'_, Audio>, layer: usize, model: Model) {
    for (index, control) in model.controls().iter().enumerate() {
        let [min, max] = control.range;
        // A continuous uniform draw, blended 60/40 toward the web preset, then quantized.
        let fraction =
            random_value(&mut context.state_mut().random_seed, 0, i16::MAX) as f32 / 32768.0;
        let value = 0.6 * f32::from(control.initial)
            + 0.4 * (f32::from(min) + fraction * f32::from(max - min));
        let steps = ((value - f32::from(min)) / f32::from(control.step) + 0.5) as i16;
        context.parameters().set(
            control_index(layer, index),
            (min + steps * control.step).min(max),
        );
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
        randomize_model_controls(context, layer, model);
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
    let seed = &mut context.state_mut().random_seed;
    let body = [
        Model::PitchSweep,
        Model::Resonator,
        Model::Modal,
        Model::TwoStage,
    ]
    .get(random_value(seed, 0, 3) as usize)
    .copied()
    .unwrap_or(Model::Modal) as i16;
    let wires = [Model::Wires, Model::SweepWires, Model::ResonantWires]
        .get(random_value(seed, 0, 2) as usize)
        .copied()
        .unwrap_or(Model::Wires) as i16;
    let wire_level = random_value(seed, 55, 85);
    let rim = layers >= 3 && random_value(seed, 0, 1) != 0;
    let rim_level = if rim { random_value(seed, 15, 45) } else { 100 };
    for layer in 0..layers {
        let (model_value, level) = match layer {
            0 => (body, 100),
            1 => (wires, wire_level),
            2 if rim => (Model::Rim as i16, rim_level),
            _ => (Model::Off as i16, 100),
        };
        let base = model_index(layer);
        if context.read_parameters().value(base) != Some(model_value) {
            context.parameters().set(base, model_value);
            // Refresh definitions even when the host defers its notification.
            update_model_controls(context, layer, false);
        }
        context.parameters().set(base + LAYER_LEVEL, level);
        for offset in [LAYER_SOLO, LAYER_MUTE] {
            let parameter = base + offset;
            if context.read_parameters().value(parameter) == Some(1) {
                context.parameters().set(parameter, 0);
            }
        }
        if let Some(&model) = context.state().models.get(layer) {
            randomize_model_controls(context, layer, model);
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
    if layer_field.is_none() && parameter != MASTER_DRIVE as i32 && parameter != GAIN as i32 {
        return;
    }
    if let Some((layer, 0)) = layer_field {
        update_model_controls(context, layer, false);
    }
    context.state_mut().dirty = true;
}

fn process_render(context: &mut Callback<'_, Audio>) {
    if let Some(slot) = context.state().in_flight {
        let resume = context.with_shared_state(|state, work| {
            if work.pending {
                return true;
            }
            if let Some(length) = work.length {
                state.player.publish(slot, length);
                state.scope = work.scope;
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
        return;
    }
    let state = context.state_mut();
    let Some(slot) = state.player.free_buffer(usize::from(state.output_buffers)) else {
        if !state.waiting_for_buffer {
            state.waiting_for_buffer = true;
            state.buffer_usage.record_wait();
        }
        return;
    };
    state.waiting_for_buffer = false;
    let mut job = Job {
        rate: state.rate,
        output_buffers: state.output_buffers,
        layer_count: state.layers as u8,
        ..Job::EMPTY
    };
    let parameters = context.read_parameters();
    job.gain = parameters.scaled(GAIN).unwrap_or(0.0);
    job.drive = parameters.scaled(MASTER_DRIVE).unwrap_or(0.0);
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
                .value(control_index(layer, index))
                .unwrap_or(control.initial);
            *decoded = control.decode(value);
        }
    }
    if context
        .with_shared_resources(|work| {
            work.job = job;
            work.plan = None;
            work.pending = false;
            work.length = None;
        })
        .is_ok()
        && context.submit_audio_worker(slot as u32).is_ok()
    {
        context.state_mut().in_flight = Some(slot);
        context.state_mut().dirty = false;
    }
}

impl Plugin for SnareLab {
    type State = State;
    type Region = ();
    type StateMemory = Sram;
    type ExtraRegions = Resources;

    const GUID: u32 = multichar(b'A', b'T', b's', b'l');
    const NAME: &'static CStr = c"SnareLab";
    const DESCRIPTION: &'static CStr = c"Layered snare drum";
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
            min: MIN_LAYERS as i32,
            max: MAX_LAYERS as i32,
            default: 3,
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
            parameters: extra_control_index(layers),
            pages: layers + 1,
            page_indices: GLOBALS + MAX_PAGE_FIELDS * layers,
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
            (Some(output_buffers), Some(layers)) => layer_slot(output_buffers, layers) + 2,
            _ => 0,
        }
    }

    fn dram_buffer_len_at(specs: &[i32], rate: u32, slot: usize) -> usize {
        let Some(output_buffers) = output_buffer_count(specs) else {
            return 0;
        };
        if layer_count(specs).is_some_and(|layers| slot >= layer_slot(output_buffers, layers)) {
            return MIX_TILE_FRAMES;
        }
        Self::dram_buffer_len(specs, rate)
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
                plan: None,
                pending: false,
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
                2,
                UNIT_CV_INPUT,
                SCALING_NONE,
            ),
            definition(
                c"Output",
                0,
                LAST_BUS as i16,
                14,
                UNIT_AUDIO_OUTPUT,
                SCALING_NONE,
            ),
            definition(c"Output Mode", 0, 1, 1, UNIT_OUTPUT_MODE, SCALING_NONE),
            definition(c"Master Drive", 0, 240, 0, UNIT_DB, SCALING_10),
            definition(c"Master Gain", -360, 120, -1, UNIT_DB, SCALING_10),
            definition(c"Randomize Layers", 0, 1, 0, UNIT_CONFIRM, SCALING_NONE),
        ];
        for (index, item) in globals.into_iter().enumerate() {
            metadata.set_parameter(index, item)?;
        }
        let indices: [u8; GLOBALS] = core::array::from_fn(|index| index as u8);
        metadata.add_page(c"SnareLab", 0, &indices)?;
        let layers = layer_count(specs)?;
        for (layer, names) in LAYER_PAGE_NAMES.iter().enumerate() {
            let base = model_index(layer);
            let model = match layer {
                0 => Model::TwoStage,
                1 => Model::ResonantWires,
                2 if layers >= 3 => Model::Rim,
                _ => Model::Off,
            };
            metadata.set_parameter(
                base,
                definition(
                    c"Model",
                    0,
                    (MODEL_NAMES.len() - 1) as i16,
                    model as i16,
                    UNIT_CONFIRM,
                    SCALING_NONE,
                ),
            )?;
            add_layer_controls(metadata, base)?;
            let controls = model.controls();
            let slots = if layer < layers {
                CONTROLS
            } else {
                LEGACY_CONTROLS
            };
            for slot in 0..slots {
                let initial = controls.get(slot).map_or(
                    if slot == LEGACY_CONTROLS { i16::MIN } else { 0 },
                    |control| control.initial,
                );
                metadata.set_parameter(
                    control_index(layer, slot),
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
            if layer < layers {
                let indices = layer_page_indices(layer);
                metadata.add_named_page(names[model as usize], 1, &indices)?;
            }
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
        handle_parameter_change(context, parameter);
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
            work.prepare_plan();
        }
        work.advance_render(slot, buffers, memory, start);
    }
    }

    fn draw(context: &mut Callback<'_, Ui>, canvas: Canvas<'_>) -> bool {
        let state = context.state();
        state.scope.draw(
            canvas,
            state.buffer_usage.peak(),
            state.buffer_usage.waits(),
            c"750 ms",
        );
        false
    }

    fn step(context: &mut Callback<'_, Audio>, buses: &mut [f32], frames: usize) {
        process_render(context);
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
    }
}

distingnt_api::export_plugin!(SnareLab);

#[cfg(test)]
mod tests;
