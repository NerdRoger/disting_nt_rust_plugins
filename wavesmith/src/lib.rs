//! Wavesmith: an editable segmented wavetable oscillator using the safe NT API.
#![no_std]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod dsp;
mod levels;
mod midi;
mod stereo;
mod telemetry;
mod ui;

use core::ffi::{c_char, CStr};
use distingnt_api::api::{
    audio::AudioBuses,
    drawing::Canvas,
    metadata::{Definition, EnumStrings},
    parameters::{Audio, ReadOnly, Ui},
    plugin::{Configuration, Context, Metadata, Plugin, Specification},
    state_memory::Sram,
};
use distingnt_api::*;

// Parameter ordinals follow page order: Oscillator, Output, Shape, voices,
// then interleaved waveform boundaries/curves. Shared controls have fixed indices.
const MIDI_CHANNEL: usize = 0;
const PITCH: usize = 1;
const FINE: usize = 2;
const LEVEL: usize = 3;
const LEFT_OUTPUT: usize = 4;
const RIGHT_OUTPUT: usize = 5;
const OUTPUT_MODE: usize = 6;
const WARP_POSITION: usize = 7;
const WARP_AMOUNT: usize = 8;
const SMOOTHING: usize = 9;
const FOLD_LIMIT: usize = 10;
const SINE_BOUNDARIES: usize = 11;
const TRIANGLE_BOUNDARIES: usize = 12;
const SQUARE_BOUNDARIES: usize = 13;
const SAWTOOTH_BOUNDARIES: usize = 14;
const ZERO_CURVES: usize = 15;
const RANDOMIZE_CURVES: usize = 16;
const RANDOMIZE_BOUNDARIES: usize = 17;
const RANDOMIZE_ALL: usize = 18;
// Trigger slots retain their existing order for edge detection and actions.
const SHAPE_TRIGGERS: [usize; 8] = [
    RANDOMIZE_ALL,
    RANDOMIZE_BOUNDARIES,
    RANDOMIZE_CURVES,
    ZERO_CURVES,
    SINE_BOUNDARIES,
    TRIANGLE_BOUNDARIES,
    SQUARE_BOUNDARIES,
    SAWTOOTH_BOUNDARIES,
];
const GLOBALS: usize = 19;
const VOICE_PARAMETERS: usize = 5;
const VOICE_COUNT: usize = 10;
const WAVEFORM_START: usize = GLOBALS + VOICE_COUNT * VOICE_PARAMETERS;
const VOICE_PAGE_NAMES: [&CStr; VOICE_COUNT] = [
    c"Voice 1",
    c"Voice 2",
    c"Voice 3",
    c"Voice 4",
    c"Voice 5",
    c"Voice 6",
    c"Voice 7",
    c"Voice 8",
    c"Voice 9",
    c"Voice 10",
];
const VOICE_PREFIXES: [&CStr; VOICE_COUNT] = [
    c"V1:", c"V2:", c"V3:", c"V4:", c"V5:", c"V6:", c"V7:", c"V8:", c"V9:", c"V10:",
];
static MIDI_CHANNELS: EnumStrings<18> = EnumStrings::new([
    c"Off", c"All", c"1", c"2", c"3", c"4", c"5", c"6", c"7", c"8", c"9", c"10", c"11", c"12",
    c"13", c"14", c"15", c"16",
]);
static ACTIVE_VALUES: EnumStrings<2> = EnumStrings::new([c"Off", c"On"]);
const fn voice_pitch_parameter(voice: usize) -> usize {
    GLOBALS + voice * VOICE_PARAMETERS
}
const fn voice_transpose_parameter(voice: usize) -> usize {
    voice_pitch_parameter(voice) + 1
}
const fn voice_fine_parameter(voice: usize) -> usize {
    voice_pitch_parameter(voice) + 2
}

const fn voice_active_parameter(voice: usize) -> usize {
    voice_pitch_parameter(voice) + 4
}

const fn voice_pan_parameter(voice: usize) -> usize {
    voice_pitch_parameter(voice) + 3
}
fn output_routing(value: impl Fn(usize) -> Option<i16>) -> stereo::Routing {
    stereo::Routing::new(
        value(LEFT_OUTPUT).unwrap_or(13),
        value(RIGHT_OUTPUT).unwrap_or(0),
    )
}

fn enabled_voices(value: impl Fn(usize) -> Option<i16>) -> [bool; VOICE_COUNT] {
    let mut enabled = [false; VOICE_COUNT];
    if output_routing(&value).enabled() {
        for (voice, on) in enabled.iter_mut().enumerate() {
            *on = value(voice_active_parameter(voice)).unwrap_or(i16::from(voice == 0)) != 0;
        }
    }
    enabled
}

fn count(specs: &[i32]) -> Option<usize> {
    let value = usize::try_from(*specs.first()?).ok()?;
    (2..=32).contains(&value).then_some(value)
}

const fn boundary_parameter(boundary: usize) -> usize {
    WAVEFORM_START + boundary * 2
}
const fn curve_parameter(segment: usize) -> usize {
    boundary_parameter(segment) + 1
}

fn random_level(seed: &mut u32) -> i16 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed % 201) as i16 - 100
}

fn editor_parameter(n: usize, segment: usize, field: usize) -> Option<usize> {
    if segment >= n {
        return None;
    }
    match field {
        0 if segment == 0 => Some(WAVEFORM_START),
        1 if segment + 1 == n => Some(WAVEFORM_START),
        0 if segment > 0 => Some(boundary_parameter(segment)),
        1 if segment + 1 < n => Some(boundary_parameter(segment + 1)),
        2 => Some(curve_parameter(segment)),
        _ => None,
    }
}

fn read_wave(n: usize, value: impl Fn(usize) -> Option<i16>) -> dsp::Wave {
    let mut wave = dsp::Wave::new(n);
    wave.boundaries[0] = value(WAVEFORM_START).unwrap_or(0).clamp(-100, 100);
    wave.smoothing = value(SMOOTHING).unwrap_or(0).clamp(0, 100);
    wave.warp_position = value(WARP_POSITION)
        .unwrap_or(dsp::WARP_POSITION_DEFAULT)
        .clamp(0, dsp::WARP_POSITION_MAX);
    wave.warp_amount = value(WARP_AMOUNT).unwrap_or(0).clamp(-100, 100);
    wave.fold_limit = value(FOLD_LIMIT)
        .unwrap_or(dsp::DEFAULT_FOLD_LIMIT)
        .clamp(0, dsp::MAX_FOLDS);
    for i in 1..n.min(dsp::MAX_SEGMENTS + 1) {
        wave.boundaries[i] = value(boundary_parameter(i)).unwrap_or(0).clamp(-100, 100);
    }
    for i in 0..n.min(dsp::MAX_SEGMENTS) {
        wave.curves[i] = value(curve_parameter(i)).unwrap_or(0).clamp(-100, 100);
    }
    wave
}

// Temporary playback profiler: average total voice-render/mix cycles per audio
// callback. Fixed 256-callback windows avoid division in the audio thread.
#[derive(Clone, Copy)]
struct RenderCycles {
    total: u64,
    callbacks: u32,
    average: u32,
}
impl RenderCycles {
    const fn new() -> Self {
        Self {
            total: 0,
            callbacks: 0,
            average: 0,
        }
    }
    fn record(&mut self, cycles: u32) {
        self.total += cycles as u64;
        self.callbacks += 1;
        if self.callbacks == 256 {
            self.average = (self.total >> 8) as u32;
            self.total = 0;
            self.callbacks = 0;
        }
    }
}

#[derive(Clone, Copy)]
struct State {
    engine: dsp::Engine,
    oscillators: [dsp::Oscillator; VOICE_COUNT],
    midi: midi::MidiState,
    cv_gains: [f32; VOICE_COUNT],
    cv_pitch: dsp::CvPitchCache,
    levels: levels::LevelControl,
    level: levels::Level,
    mode_transition: levels::ModeTransition,
    right_mode_transition: levels::ModeTransition,
    routing: Option<stereo::Routing>,
    pans: [stereo::Pan; VOICE_COUNT],
    pan_slew: f32,
    mix: [f32; levels::CHUNK],
    right_mix: [f32; levels::CHUNK],
    voice_mix: [f32; levels::CHUNK],
    render_cycles: RenderCycles,
    telemetry: telemetry::Telemetry,
    count: usize,
    sample_rate: f32,
    ui: ui::UiState,
    random_seed: u32,
    trigger_high: [bool; 8],
    grayed_for_midi: Option<bool>,
    grayed_for_stereo: Option<bool>,
}

impl State {
    fn set_midi_channel(&mut self, setting: u8, enabled: &[bool; VOICE_COUNT]) {
        if self.midi.set_channel(setting) {
            self.oscillators = [dsp::Oscillator::new(); VOICE_COUNT];
            for (gain, &on) in self.cv_gains.iter_mut().zip(enabled) {
                *gain = if on { 1.0 } else { 0.0 };
            }
            self.levels.restart_for_mode_change();
            self.mode_transition.start();
            self.right_mode_transition.start();
        }
    }
    fn set_routing(&mut self, routing: stereo::Routing, pan_positions: &[i16; VOICE_COUNT]) {
        let old = self.routing;
        let changed = old != Some(routing);
        let first_bus = if routing.stereo() {
            routing.left
        } else {
            routing.primary()
        };
        if let Some(previous) = old.filter(|&previous| previous != routing) {
            let left = self.mode_transition.current();
            let right = self.right_mode_transition.current();
            let old_first_bus = if previous.stereo() {
                previous.left
            } else {
                previous.primary()
            };
            let connected = |bus: i16| bus != 0 && (bus == previous.left || bus == previous.right);
            if first_bus == old_first_bus && previous.stereo() == routing.stereo() {
                // Preserve the existing waveform and any MIDI transition.
            } else if connected(first_bus) {
                self.mode_transition
                    .adopt(previous.sample_on(first_bus, left, right));
            } else {
                self.mode_transition.start_from(0.0);
            }
            if routing.stereo() {
                if previous.stereo() && routing.right == previous.right {
                    // A change on the other side does not affect this channel.
                } else if connected(routing.right) {
                    self.right_mode_transition.adopt(previous.sample_on(
                        routing.right,
                        left,
                        right,
                    ));
                } else {
                    self.right_mode_transition.start_from(0.0);
                }
            }
            // Live rerouting preserves the RMS history, settled gain and limiter
            // hold. Only reconnecting after silence needs startup acquisition.
            if !previous.enabled() && routing.enabled() {
                self.levels.restart_for_mode_change();
            }
        }
        for (pan, &position) in self.pans.iter_mut().zip(pan_positions) {
            let gains = pan.gains();
            let immediate = old.is_none() || old.is_some_and(|old| !old.enabled());
            if routing.stereo() {
                pan.prepare(position, immediate);
                if changed && old.is_some_and(|old| old.enabled() && !old.stereo()) {
                    let previous_bus = old.map(|old| old.primary()).unwrap_or(0);
                    pan.start_from(
                        if routing.left == previous_bus {
                            gains.0
                        } else {
                            0.0
                        },
                        if routing.right == previous_bus {
                            gains.0
                        } else {
                            0.0
                        },
                    );
                }
            } else {
                pan.mono(immediate);
                if changed && old.is_some_and(|old| old.stereo()) {
                    let previous = old.unwrap_or(routing);
                    let gain = if first_bus == previous.left {
                        gains.0
                    } else if first_bus == previous.right {
                        gains.1
                    } else {
                        0.0
                    };
                    pan.start_from(gain, 0.0);
                }
            }
        }
        self.routing = Some(routing);
    }
}

type Callback<'a, Mode> = Context<'a, State, dsp::Block, Mode, Sram>;
struct Wavesmith;

fn sync_midi_parameter_graying(context: &mut Callback<'_, Audio>) {
    let midi = context
        .read_parameters()
        .value(MIDI_CHANNEL)
        .unwrap_or(0)
        .clamp(0, 17)
        != 0;
    if context.state().grayed_for_midi == Some(midi) {
        return;
    }
    // Publish before calling the host so synchronous reentry cannot repeat it.
    context.state_mut().grayed_for_midi = Some(midi);
    let mut parameters = context.parameters();
    let mut applied = parameters.gray(PITCH, midi).is_some();
    for voice in 0..VOICE_COUNT {
        applied &= parameters
            .gray(voice_pitch_parameter(voice), midi)
            .is_some();
        applied &= parameters
            .gray(voice_transpose_parameter(voice), midi)
            .is_some();
    }
    if !applied {
        context.state_mut().grayed_for_midi = None;
    }
}

fn sync_parameter_graying(context: &mut Callback<'_, Audio>) {
    sync_midi_parameter_graying(context);
    let stereo = output_routing(|p| context.read_parameters().value(p)).stereo();
    if context.state().grayed_for_stereo == Some(stereo) {
        return;
    }
    context.state_mut().grayed_for_stereo = Some(stereo);
    let mut parameters = context.parameters();
    let mut applied = true;
    for voice in 0..VOICE_COUNT {
        applied &= parameters
            .gray(voice_pan_parameter(voice), !stereo)
            .is_some();
    }
    if !applied {
        context.state_mut().grayed_for_stereo = None;
    }
}

fn definition(
    name: &'static CStr,
    min: i16,
    max: i16,
    default: i16,
    unit: u8,
) -> Option<Definition> {
    Definition::new(name, min, max, default, unit, SCALING_NONE, None)
}

const BOUNDARY_NAMES: [&CStr; 31] = [
    c"Boundary 01",
    c"Boundary 02",
    c"Boundary 03",
    c"Boundary 04",
    c"Boundary 05",
    c"Boundary 06",
    c"Boundary 07",
    c"Boundary 08",
    c"Boundary 09",
    c"Boundary 10",
    c"Boundary 11",
    c"Boundary 12",
    c"Boundary 13",
    c"Boundary 14",
    c"Boundary 15",
    c"Boundary 16",
    c"Boundary 17",
    c"Boundary 18",
    c"Boundary 19",
    c"Boundary 20",
    c"Boundary 21",
    c"Boundary 22",
    c"Boundary 23",
    c"Boundary 24",
    c"Boundary 25",
    c"Boundary 26",
    c"Boundary 27",
    c"Boundary 28",
    c"Boundary 29",
    c"Boundary 30",
    c"Boundary 31",
];
const CURVE_NAMES: [&CStr; 32] = [
    c"Curve 01",
    c"Curve 02",
    c"Curve 03",
    c"Curve 04",
    c"Curve 05",
    c"Curve 06",
    c"Curve 07",
    c"Curve 08",
    c"Curve 09",
    c"Curve 10",
    c"Curve 11",
    c"Curve 12",
    c"Curve 13",
    c"Curve 14",
    c"Curve 15",
    c"Curve 16",
    c"Curve 17",
    c"Curve 18",
    c"Curve 19",
    c"Curve 20",
    c"Curve 21",
    c"Curve 22",
    c"Curve 23",
    c"Curve 24",
    c"Curve 25",
    c"Curve 26",
    c"Curve 27",
    c"Curve 28",
    c"Curve 29",
    c"Curve 30",
    c"Curve 31",
    c"Curve 32",
];

impl Plugin for Wavesmith {
    type State = State;
    type Region = dsp::Block;
    type StateMemory = Sram;
    const GUID: u32 = multichar(b'N', b'R', b'w', b's');
    const NAME: &'static CStr = c"Wavesmith";
    const DESCRIPTION: &'static CStr = c"Sculpt a live oscillator from curved segments";
    const TAGS: u32 = TAG_INSTRUMENT;
    const SPECIFICATIONS: &'static [Specification] = &[Specification {
        name: c"Segments",
        min: 2,
        max: 32,
        default: 8,
        kind: TYPE_GENERIC,
    }];
    const CUSTOM_UI: u32 = POT_L
        | POT_C
        | POT_R
        | POT_BUTTON_L
        | POT_BUTTON_C
        | POT_BUTTON_R
        | ENCODER_L
        | ENCODER_R
        | ENCODER_BUTTON_R;
    const INITIALIZATION_IN_DRAM: bool = true;
    const MIDI: bool = true;
    const MIDI_SYSEX: bool = true;
    const PARAMETER_TEXT: bool = true;

    fn configuration(specs: &[i32]) -> Option<Configuration> {
        let n = count(specs)?;
        let parameters = GLOBALS + n * 2 + VOICE_COUNT * VOICE_PARAMETERS;
        Some(Configuration {
            parameters,
            pages: 4 + VOICE_COUNT,
            page_indices: parameters + 1,
            regions: dsp::blocks(dsp::DEFAULT_TABLES),
        })
    }

    #[inline(always)]
    fn initialize_state(
        destination: &mut distingnt_api::api::plugin::RegionInitializer<'_, State>,
        specs: &[i32],
        rate: u32,
    ) -> Option<()> {
        let n = count(specs)?;
        if rate == 0 {
            return None;
        }
        distingnt_api::initialize_region!(
            destination,
            State {
                engine: dsp::Engine::new(n, rate),
                oscillators: [dsp::Oscillator::new(); VOICE_COUNT],
                midi: midi::MidiState::new(rate),
                cv_gains: [0.0; VOICE_COUNT],
                cv_pitch: dsp::CvPitchCache::new(),
                levels: levels::LevelControl::new(rate),
                level: levels::Level::new(),
                mode_transition: levels::ModeTransition::new(rate),
                right_mode_transition: levels::ModeTransition::new(rate),
                routing: None,
                pans: [stereo::Pan::new(); VOICE_COUNT],
                pan_slew: (1000.0 / rate as f32).min(1.0),
                mix: [0.0; levels::CHUNK],
                right_mix: [0.0; levels::CHUNK],
                voice_mix: [0.0; levels::CHUNK],
                render_cycles: RenderCycles::new(),
                telemetry: telemetry::Telemetry::new(),
                count: n,
                sample_rate: rate as f32,
                ui: ui::UiState::new(),
                random_seed: 0x9e37_79b9 ^ rate ^ n as u32,
                trigger_high: [false; 8],
                grayed_for_midi: None,
                grayed_for_stereo: None,
            }
        );
        Some(())
    }

    fn initialize_region(
        destination: &mut distingnt_api::api::plugin::RegionInitializer<'_, dsp::Block>,
        _: &[i32],
        _: usize,
    ) -> Option<()> {
        dsp::Block::initialize(destination);
        Some(())
    }

    fn metadata(specs: &[i32], metadata: &mut Metadata<'_>) -> Option<()> {
        let n = count(specs)?;
        for (index, d) in [
            (
                LEFT_OUTPUT,
                definition(c"Left Output", 0, LAST_BUS as i16, 13, UNIT_AUDIO_OUTPUT),
            ),
            (
                RIGHT_OUTPUT,
                definition(c"Right Output", 0, LAST_BUS as i16, 0, UNIT_AUDIO_OUTPUT),
            ),
            (
                OUTPUT_MODE,
                definition(c"Output mode", 0, 1, 0, UNIT_OUTPUT_MODE),
            ),
            (PITCH, definition(c"Pitch", 0, 127, 57, UNIT_MIDI_NOTE)),
            (FINE, definition(c"Fine tune", -100, 100, 0, UNIT_CENTS)),
            (SMOOTHING, definition(c"Smoothing", 0, 100, 0, UNIT_PERCENT)),
            (
                LEVEL,
                definition(
                    c"Level",
                    levels::LEVEL_MIN,
                    0,
                    levels::LEVEL_DEFAULT,
                    UNIT_DB_MIN_INF,
                ),
            ),
        ] {
            metadata.set_parameter(index, d?)?;
        }
        metadata.add_page(
            c"Oscillator",
            1,
            &[MIDI_CHANNEL as u8, PITCH as u8, FINE as u8, LEVEL as u8],
        )?;
        metadata.add_page(
            c"Output",
            2,
            &[LEFT_OUTPUT as u8, RIGHT_OUTPUT as u8, OUTPUT_MODE as u8],
        )?;
        metadata.add_page(
            c"Shape",
            3,
            &[
                WARP_POSITION as u8,
                WARP_AMOUNT as u8,
                SMOOTHING as u8,
                FOLD_LIMIT as u8,
                SINE_BOUNDARIES as u8,
                TRIANGLE_BOUNDARIES as u8,
                SQUARE_BOUNDARIES as u8,
                SAWTOOTH_BOUNDARIES as u8,
                ZERO_CURVES as u8,
                RANDOMIZE_CURVES as u8,
                RANDOMIZE_BOUNDARIES as u8,
                RANDOMIZE_ALL as u8,
            ],
        )?;
        for (voice, &page_name) in VOICE_PAGE_NAMES.iter().take(VOICE_COUNT).enumerate() {
            let cv = voice_pitch_parameter(voice);
            metadata.set_parameter(
                cv,
                definition(c"Pitch CV", 0, LAST_BUS as i16, 0, UNIT_CV_INPUT)?,
            )?;
            let transpose = voice_transpose_parameter(voice);
            let fine = voice_fine_parameter(voice);
            metadata.set_parameter(
                transpose,
                definition(c"Transpose", -48, 48, 0, UNIT_SEMITONES)?,
            )?;
            metadata.set_parameter(fine, definition(c"Fine tune", -100, 100, 0, UNIT_CENTS)?)?;
            let pan = voice_pan_parameter(voice);
            metadata.set_parameter(pan, definition(c"Pan", 0, 100, 50, UNIT_PERCENT)?)?;
            let active = voice_active_parameter(voice);
            metadata.set_parameter(
                active,
                Definition::new(
                    c"Active",
                    0,
                    1,
                    if voice == 0 { 1 } else { 0 },
                    UNIT_ENUM,
                    SCALING_NONE,
                    Some(ACTIVE_VALUES.table()),
                )?,
            )?;
            // Shared group retains the selected parameter row between voices.
            metadata.add_page(
                page_name,
                4,
                &[
                    cv as u8,
                    transpose as u8,
                    fine as u8,
                    pan as u8,
                    active as u8,
                ],
            )?;
        }
        for (i, name) in BOUNDARY_NAMES.iter().enumerate().take(n - 1) {
            metadata.set_parameter(
                boundary_parameter(i + 1),
                definition(name, -100, 100, dsp::default_level(i + 1, n), UNIT_PERCENT)?,
            )?;
        }
        for (i, name) in CURVE_NAMES.iter().enumerate().take(n) {
            metadata.set_parameter(
                curve_parameter(i),
                definition(name, -100, 100, 0, UNIT_PERCENT)?,
            )?;
        }
        // Both ends of the cycle reference the same editable seam boundary.
        metadata.set_parameter(
            WAVEFORM_START,
            definition(c"Boundary 00", -100, 100, 0, UNIT_PERCENT)?,
        )?;
        for (parameter, name) in SHAPE_TRIGGERS.into_iter().zip([
            c"Randomize All",
            c"Randomize Boundaries",
            c"Randomize Curves",
            c"Zero Curves",
            c"Sine Boundaries",
            c"Triangle Boundaries",
            c"Square Boundaries",
            c"Sawtooth Boundaries",
        ]) {
            metadata.set_parameter(parameter, definition(name, 0, 1, 0, UNIT_NONE)?)?;
        }
        metadata.set_parameter(
            WARP_POSITION,
            definition(
                c"Warp Position",
                0,
                dsp::WARP_POSITION_MAX,
                dsp::WARP_POSITION_DEFAULT,
                UNIT_NONE,
            )?,
        )?;
        metadata.set_parameter(
            WARP_AMOUNT,
            definition(c"Warp Amount", -100, 100, 0, UNIT_PERCENT)?,
        )?;
        metadata.set_parameter(
            FOLD_LIMIT,
            definition(
                c"Fold Limit",
                0,
                dsp::MAX_FOLDS,
                dsp::DEFAULT_FOLD_LIMIT,
                UNIT_NONE,
            )?,
        )?;
        metadata.set_parameter(
            MIDI_CHANNEL,
            Definition::new(
                c"MIDI Channel",
                0,
                17,
                0,
                UNIT_ENUM,
                SCALING_NONE,
                Some(MIDI_CHANNELS.table()),
            )?,
        )?;
        // Segment count does not add pages; even ten voices stay below 32 pages.
        // Walk the shape in time order, repeating Boundary 00 at the end.
        let mut indices = [0u8; dsp::MAX_SEGMENTS * 2 + 1];
        *indices.first_mut()? = WAVEFORM_START as u8;
        for segment in 0..n {
            *indices.get_mut(segment * 2 + 1)? = curve_parameter(segment) as u8;
            *indices.get_mut(segment * 2 + 2)? =
                editor_parameter(n, segment, 1).unwrap_or(WAVEFORM_START) as u8;
        }
        metadata.add_page(c"Waveform", 5, indices.get(..n * 2 + 1)?)?;
        Some(())
    }

    fn parameter_changed(context: &mut Callback<'_, Audio>, parameter: i32) {
        let n = context.state().count;
        if parameter == MIDI_CHANNEL as i32 {
            sync_parameter_graying(context);
            let setting = context
                .read_parameters()
                .value(MIDI_CHANNEL)
                .unwrap_or(0)
                .clamp(0, 17) as u8;
            let enabled = enabled_voices(|index| context.read_parameters().value(index));
            let state = context.state_mut();
            state.set_midi_channel(setting, &enabled);
            return;
        }
        if parameter == LEFT_OUTPUT as i32 || parameter == RIGHT_OUTPUT as i32 {
            sync_parameter_graying(context);
            if !output_routing(|p| context.read_parameters().value(p)).enabled() {
                context.state_mut().midi.sync_enabled(&[false; VOICE_COUNT]);
            }
            return;
        }
        let triggers = SHAPE_TRIGGERS;
        let Some(trigger) = triggers.iter().position(|&index| parameter == index as i32) else {
            return;
        };
        let high = context
            .read_parameters()
            .value(triggers[trigger])
            .unwrap_or(0)
            != 0;
        let rising = high && !context.state().trigger_high[trigger];
        context.state_mut().trigger_high[trigger] = high;
        if !rising {
            return;
        }
        if trigger >= 4 {
            let shape = match trigger {
                4 => dsp::BoundaryShape::Sine,
                5 => dsp::BoundaryShape::Triangle,
                6 => dsp::BoundaryShape::Square,
                _ => dsp::BoundaryShape::Sawtooth,
            };
            let _ = context
                .parameters()
                .set(WAVEFORM_START, dsp::preset_boundary(shape, 0, n));
            for boundary in 1..n {
                let _ = context.parameters().set(
                    boundary_parameter(boundary),
                    dsp::preset_boundary(shape, boundary, n),
                );
            }
            return;
        }
        // Publish real host parameters so the editor, modulation and presets
        // all see the randomized shape. Release the state borrow before setters.
        let mut seed = context.state().random_seed;
        if trigger == 0 || trigger == 1 {
            let _ = context
                .parameters()
                .set(WAVEFORM_START, random_level(&mut seed));
            for boundary in 1..n {
                let _ = context
                    .parameters()
                    .set(boundary_parameter(boundary), random_level(&mut seed));
            }
        }
        if trigger == 0 || trigger == 2 || trigger == 3 {
            for segment in 0..n {
                let value = if trigger == 3 {
                    0
                } else {
                    random_level(&mut seed)
                };
                let _ = context.parameters().set(curve_parameter(segment), value);
            }
        }
        context.state_mut().random_seed = seed;
    }

    fn prefix(_context: &mut Callback<'_, ReadOnly>, parameter: i32, buffer: &mut [c_char]) -> i32 {
        let Ok(parameter) = usize::try_from(parameter) else {
            return 0;
        };
        for (voice, &prefix) in VOICE_PREFIXES.iter().take(VOICE_COUNT).enumerate() {
            if [
                voice_pitch_parameter(voice),
                voice_transpose_parameter(voice),
                voice_fine_parameter(voice),
                voice_pan_parameter(voice),
                voice_active_parameter(voice),
            ]
            .contains(&parameter)
            {
                let bytes = prefix.to_bytes_with_nul();
                let Some(output) = buffer.get_mut(..bytes.len()) else {
                    return 0;
                };
                for (out, &byte) in output.iter_mut().zip(bytes) {
                    *out = byte as c_char;
                }
                return bytes.len() as i32 - 1;
            }
        }
        0
    }

    fn midi_sysex(bytes: &[u8]) {
        telemetry::control(bytes);
    }

    fn midi_message(context: &mut Callback<'_, Audio>, status: u8, data1: u8, data2: u8) {
        sync_parameter_graying(context);
        let p = context.read_parameters();
        let setting = p.value(MIDI_CHANNEL).unwrap_or(0).clamp(0, 17) as u8;
        let enabled = enabled_voices(|index| p.value(index));
        let state = context.state_mut();
        state.set_midi_channel(setting, &enabled);
        state.midi.message(status, data1, data2, &enabled);
    }

    fn step(context: &mut Callback<'_, Audio>, samples: &mut [f32], frames: usize) {
        let callback_start = distingnt_api::api::cpu_cycle_count();
        sync_parameter_graying(context);
        let n = context.state().count.clamp(2, 32);
        let p = context.read_parameters();
        let wave = read_wave(n, |index| p.value(index));
        let note = p.value(PITCH).unwrap_or(57).clamp(0, 127) as f32;
        let fine = p.value(FINE).unwrap_or(0).clamp(-100, 100) as f32 / 1200.0;
        let level_db = p.value(LEVEL).unwrap_or(levels::LEVEL_DEFAULT);
        let midi_setting = p.value(MIDI_CHANNEL).unwrap_or(0).clamp(0, 17) as u8;
        let Some(mut buses) = AudioBuses::new(samples, frames) else {
            return;
        };
        let routing = output_routing(|index| p.value(index));
        let output_enabled = routing.enabled();
        let mut pan_positions = [50; VOICE_COUNT];
        if routing.stereo() {
            for (voice, pan) in pan_positions.iter_mut().enumerate() {
                *pan = p.value(voice_pan_parameter(voice)).unwrap_or(50);
            }
        }
        let replace = p.value(OUTPUT_MODE).unwrap_or(0) != 0;
        let enabled = enabled_voices(|index| p.value(index));
        let mut render_voice = [false; VOICE_COUNT];
        for (voice, (&on, render)) in enabled
            .iter()
            .zip(render_voice.iter_mut())
            .take(VOICE_COUNT)
            .enumerate()
        {
            *render = output_enabled
                && (on
                    || if midi_setting == 0 {
                        context
                            .state()
                            .cv_gains
                            .get(voice)
                            .is_some_and(|&gain| gain > 0.0)
                    } else {
                        context
                            .state()
                            .midi
                            .voices
                            .get(voice)
                            .is_some_and(|v| v.sounding())
                    });
        }
        let mut pitches = [0.0f32; VOICE_COUNT];
        let mut cv_inputs = [(0i16, 0.0f32); VOICE_COUNT];
        // Snapshot tuning/CV before clearing or writing the shared destination.
        if output_enabled {
            for (voice, (pitch, &render)) in pitches
                .iter_mut()
                .zip(&render_voice)
                .take(VOICE_COUNT)
                .enumerate()
            {
                if !render {
                    continue;
                }
                let voice_fine = p
                    .value(voice_fine_parameter(voice))
                    .unwrap_or(0)
                    .clamp(-100, 100) as f32
                    / 1200.0;
                if midi_setting != 0 {
                    // MIDI ignores global Pitch, Pitch CV and per-voice Transpose.
                    *pitch = fine + voice_fine;
                    continue;
                }
                let transpose = p
                    .value(voice_transpose_parameter(voice))
                    .unwrap_or(0)
                    .clamp(-48, 48) as f32
                    / 12.0;
                let bus = p.value(voice_pitch_parameter(voice)).unwrap_or(0);
                let cv = buses.read_first(bus);
                let cv = if cv.is_finite() {
                    cv.clamp(-10.0, 10.0)
                } else {
                    0.0
                };
                if let Some(input) = cv_inputs.get_mut(voice) {
                    *input = (bus, cv);
                }
                *pitch = (note - 69.0) / 12.0 + fine + voice_fine + transpose;
            }
        }
        let processing = context.processing();
        let state = processing.state;
        let level = state.level.amplitude(level_db);
        let memory = processing.regions;
        state.set_midi_channel(midi_setting, &enabled);
        state.set_routing(routing, &pan_positions);
        if output_enabled {
            state.midi.release_disabled(&enabled);
        } else {
            state.midi.sync_enabled(&enabled);
            state.cv_gains = [0.0; VOICE_COUNT];
        }
        state.midi.begin_block();
        let pitch_bends = &state.midi.pitch_bends;
        state.cv_pitch.begin_block();
        for ((((midi, oscillator), pitch), &render), &(bus, cv)) in state
            .midi
            .voices
            .iter_mut()
            .zip(state.oscillators.iter_mut())
            .zip(pitches.iter_mut())
            .zip(&render_voice)
            .zip(&cv_inputs)
            .take(VOICE_COUNT)
        {
            if !render {
                *pitch = 0.0;
                continue;
            }
            if midi_setting != 0 {
                if !midi.sounding() {
                    *pitch = 0.0;
                    continue;
                }
                let octave =
                    (midi.pitch() as f32 - 69.0) / 12.0 + midi.bend_octaves(pitch_bends) + *pitch;
                let frequency = oscillator.frequency(octave.clamp(-16.0, 16.0));
                if midi.take_retune() {
                    oscillator.tune_when_silent(frequency, state.sample_rate);
                }
                *pitch = frequency;
                if frames > 0 {
                    oscillator.prepare(frequency, state.sample_rate, level);
                }
            } else {
                let factor = state.cv_pitch.factor(bus, cv);
                if frames > 0 {
                    oscillator.prepare_cv(*pitch, factor, state.sample_rate, level);
                }
                // Only used below as an enabled-voice marker in CV mode.
                *pitch = 1.0;
            }
        }
        let setup_end = distingnt_api::api::cpu_cycle_count();
        state.engine.request(wave);
        state.engine.advance(memory);
        let build_end = distingnt_api::api::cpu_cycle_count();
        let mut render_cycles = 0u32;
        let mut level_cycles = 0u32;
        let mut output_cycles = 0u32;
        if output_enabled {
            let gate_step = state.midi.gate_step;
            for (chunk_index, start) in (0..frames).step_by(levels::CHUNK).enumerate() {
                let end = start.saturating_add(levels::CHUNK).min(frames);
                let length = end - start;
                let Some(mix) = state.mix.get_mut(..length) else {
                    return;
                };
                mix.fill(0.0);
                let Some(right_mix) = state.right_mix.get_mut(..length) else {
                    return;
                };
                right_mix.fill(0.0);
                let Some(voice_mix) = state.voice_mix.get_mut(..length) else {
                    return;
                };
                let mut voice_power = 0.0;
                let mut gate_power = 0.0;
                let render_start = distingnt_api::api::cpu_cycle_count();
                for (((((oscillator, midi), &frequency), cv_gain), &on), pan) in state
                    .oscillators
                    .iter_mut()
                    .zip(state.midi.voices.iter_mut())
                    .zip(&pitches)
                    .zip(state.cv_gains.iter_mut())
                    .zip(&enabled)
                    .zip(state.pans.iter_mut())
                    .take(VOICE_COUNT)
                {
                    if frequency == 0.0 {
                        continue;
                    }
                    voice_mix.fill(0.0);

                    if (if midi_setting == 0 {
                        on && *cv_gain == 1.0
                    } else {
                        midi.gate_is_steady()
                    }) && oscillator.render_open_gate(&state.engine, memory, voice_mix)
                    {
                        gate_power += length as f32;
                    } else {
                        for (frame, sample) in voice_mix.iter_mut().enumerate() {
                            let gate = if midi_setting == 0 {
                                *cv_gain = if on {
                                    (*cv_gain + gate_step).min(1.0)
                                } else {
                                    (*cv_gain - gate_step).max(0.0)
                                };
                                *cv_gain
                            } else {
                                midi.next_gain(gate_step)
                            };
                            if gate == 0.0 {
                                break;
                            }
                            gate_power += gate * gate;
                            *sample = gate
                                * oscillator.tick_prepared(
                                    &state.engine,
                                    memory,
                                    chunk_index * levels::CHUNK + frame,
                                );
                        }
                    }
                    if routing.stereo() || pan.moving() {
                        voice_power += pan.mix(voice_mix, mix, right_mix, state.pan_slew);
                    } else {
                        for (sum, &sample) in mix.iter_mut().zip(voice_mix.iter()) {
                            *sum += sample;
                            voice_power += sample * sample;
                        }
                    }
                }
                let render_end = distingnt_api::api::cpu_cycle_count();
                render_cycles = render_cycles.wrapping_add(render_end.wrapping_sub(render_start));
                // Protect only Wavesmith's contribution; Add mode preserves
                // upstream bus audio exactly, including any existing peaks.
                let ceiling = if level > 0.0 {
                    levels::PEAK_CEILING
                } else {
                    0.0
                };
                if routing.stereo() {
                    state.levels.process_stereo(
                        mix,
                        right_mix,
                        voice_power,
                        gate_power / length as f32,
                        ceiling,
                    );
                } else {
                    state
                        .levels
                        .process(mix, voice_power, gate_power / length as f32, ceiling);
                }
                let level_end = distingnt_api::api::cpu_cycle_count();
                level_cycles = level_cycles.wrapping_add(level_end.wrapping_sub(render_end));
                state.mode_transition.apply(mix);
                let first_bus = if routing.stereo() {
                    routing.left
                } else {
                    routing.primary()
                };
                if let Some(output) = buses
                    .bus_mut(first_bus)
                    .and_then(|bus| bus.get_mut(start..end))
                {
                    for (out, &sample) in output.iter_mut().zip(mix.iter()) {
                        if replace {
                            *out = sample;
                        } else {
                            *out += sample;
                        }
                    }
                }
                if routing.stereo() {
                    state.right_mode_transition.apply(right_mix);
                    if let Some(output) = buses
                        .bus_mut(routing.right)
                        .and_then(|bus| bus.get_mut(start..end))
                    {
                        for (out, &sample) in output.iter_mut().zip(right_mix.iter()) {
                            if replace {
                                *out = sample;
                            } else {
                                *out += sample;
                            }
                        }
                    }
                } else {
                    state.right_mode_transition.silence();
                }
                output_cycles = output_cycles
                    .wrapping_add(distingnt_api::api::cpu_cycle_count().wrapping_sub(level_end));
            }
        } else {
            state.levels.reset();
            state.mode_transition.silence();
            state.right_mode_transition.silence();
        }
        state.render_cycles.record(render_cycles);
        // Shared edit time advances once per block even when every output is
        // None, so silent editing cannot stall an unfinished crossfade/build.
        state.engine.advance_time(frames);
        let callback_end = distingnt_api::api::cpu_cycle_count();
        let packet = state.telemetry.record(
            render_cycles,
            frames,
            state.sample_rate as u32,
            state.render_cycles.average,
            pitches.iter().filter(|&&f| f > 0.0).count() as u8,
            midi_setting != 0,
            routing.stereo(),
            [
                callback_end.wrapping_sub(callback_start),
                setup_end.wrapping_sub(callback_start),
                build_end.wrapping_sub(setup_end),
                level_cycles,
                output_cycles,
            ],
        );
        if let Some(mut packet) = packet {
            telemetry::encode(
                context.algorithm_index().unwrap_or(u32::MAX) as u64,
                &mut packet[6..11],
            );
            context.send_midi_sysex(DESTINATION_USB, &packet);
        }
    }

    fn draw(context: &mut Callback<'_, Ui>, canvas: Canvas<'_>) -> bool {
        ui::draw(context, canvas);
        // The waveform editor owns the controls while this view is active.
        true
    }
    fn custom_ui(context: &mut Callback<'_, Ui>, data: &NtUiData) {
        ui::handle(context, data);
    }
}

distingnt_api::export_plugin!(Wavesmith);

#[cfg(test)]
mod tests;
