//! Wavesmith: an editable segmented wavetable oscillator using the safe NT API.
#![no_std]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod dsp;
mod ui;

use core::ffi::CStr;
use distingnt_api::api::{
    audio::AudioBuses,
    drawing::Canvas,
    metadata::Definition,
    parameters::{Audio, Ui},
    plugin::{Configuration, Context, Metadata, Plugin, Specification},
    state_memory::Sram,
};
use distingnt_api::*;

const OUTPUT: usize = 0;
const OUTPUT_MODE: usize = 1;
const PITCH: usize = 2;
const FINE: usize = 3;
const PITCH_CV: usize = 4;
const SMOOTHING: usize = 5;
const LEVEL: usize = 6;
const GLOBALS: usize = 7;

fn count(specs: &[i32]) -> Option<usize> {
    let value = usize::try_from(*specs.first()?).ok()?;
    (2..=32).contains(&value).then_some(value)
}

fn table_count(specs: &[i32]) -> Option<usize> {
    let value = usize::try_from(*specs.get(1)?).ok()?;
    (1..=dsp::MAX_TABLES).contains(&value).then_some(value)
}

const fn boundary_parameter(boundary: usize) -> usize {
    GLOBALS + boundary - 1
}

const fn curve_parameter(n: usize, segment: usize) -> usize {
    GLOBALS + n - 1 + segment
}

const fn seam_parameter(n: usize) -> usize {
    GLOBALS + n * 2 - 1
}

const fn randomize_parameter(n: usize) -> usize {
    GLOBALS + n * 2
}

const fn centroid_parameter(n: usize) -> usize {
    randomize_parameter(n) + 1
}
const fn strength_parameter(n: usize) -> usize {
    randomize_parameter(n) + 2
}
const fn randomize_boundaries_parameter(n: usize) -> usize {
    randomize_parameter(n) + 3
}
const fn randomize_curves_parameter(n: usize) -> usize {
    randomize_parameter(n) + 4
}
const fn zero_curves_parameter(n: usize) -> usize {
    randomize_parameter(n) + 5
}
const fn fold_limit_parameter(n: usize) -> usize {
    randomize_parameter(n) + 10
}

fn shape_triggers(n: usize) -> [usize; 8] {
    [
        randomize_parameter(n),
        randomize_boundaries_parameter(n),
        randomize_curves_parameter(n),
        zero_curves_parameter(n),
        randomize_parameter(n) + 6,
        randomize_parameter(n) + 7,
        randomize_parameter(n) + 8,
        randomize_parameter(n) + 9,
    ]
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
        0 if segment == 0 => Some(seam_parameter(n)),
        1 if segment + 1 == n => Some(seam_parameter(n)),
        0 if segment > 0 => Some(boundary_parameter(segment)),
        1 if segment + 1 < n => Some(boundary_parameter(segment + 1)),
        2 => Some(curve_parameter(n, segment)),
        _ => None,
    }
}

fn read_wave(n: usize, value: impl Fn(usize) -> Option<i16>) -> dsp::Wave {
    let mut wave = dsp::Wave::new(n);
    wave.boundaries[0] = value(seam_parameter(n)).unwrap_or(0).clamp(-100, 100);
    wave.smoothing = value(SMOOTHING).unwrap_or(0).clamp(0, 100);
    wave.centroid = value(centroid_parameter(n))
        .unwrap_or(dsp::CENTROID_DEFAULT)
        .clamp(0, dsp::CENTROID_MAX);
    wave.strength = value(strength_parameter(n)).unwrap_or(0).clamp(-100, 100);
    wave.fold_limit = value(fold_limit_parameter(n))
        .unwrap_or(dsp::DEFAULT_FOLD_LIMIT)
        .clamp(0, dsp::MAX_FOLDS);
    for i in 1..n.min(dsp::MAX_SEGMENTS + 1) {
        wave.boundaries[i] = value(boundary_parameter(i)).unwrap_or(0).clamp(-100, 100);
    }
    for i in 0..n.min(dsp::MAX_SEGMENTS) {
        wave.curves[i] = value(curve_parameter(n, i)).unwrap_or(0).clamp(-100, 100);
    }
    wave
}

#[derive(Clone, Copy)]
struct State {
    engine: dsp::Engine,
    count: usize,
    sample_rate: f32,
    ui: ui::UiState,
    random_seed: u32,
    trigger_high: [bool; 8],
}

type Callback<'a, Mode> = Context<'a, State, dsp::Block, Mode, Sram>;
struct Wavesmith;

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
    type Voice = dsp::Block;
    type StateMemory = Sram;
    const GUID: u32 = multichar(b'N', b'R', b'w', b's');
    const NAME: &'static CStr = c"Wavesmith";
    const DESCRIPTION: &'static CStr = c"Sculpt a live oscillator from curved segments";
    const TAGS: u32 = TAG_INSTRUMENT;
    const SPECIFICATIONS: &'static [Specification] = &[
        Specification {
            name: c"Segments",
            min: 2,
            max: 32,
            default: 8,
            kind: TYPE_GENERIC,
        },
        Specification {
            name: c"Anti-alias tables",
            min: 1,
            max: dsp::MAX_TABLES as i32,
            default: dsp::DEFAULT_TABLES as i32,
            kind: TYPE_GENERIC,
        },
    ];
    const CUSTOM_UI: u32 = POT_L | POT_C | POT_R | ENCODER_L | ENCODER_R | ENCODER_BUTTON_R;
    const INITIALIZATION_IN_DRAM: bool = true;

    fn configuration(specs: &[i32]) -> Option<Configuration> {
        let n = count(specs)?;
        Some(Configuration {
            parameters: GLOBALS + n * 2 + 11,
            pages: 3,
            page_indices: GLOBALS + n * 2 + 12,
            voices: dsp::blocks(table_count(specs)?),
        })
    }

    fn create(specs: &[i32], rate: u32) -> Option<State> {
        let n = count(specs)?;
        if rate == 0 {
            return None;
        }
        Some(State {
            engine: dsp::Engine::with_tables(n, rate, table_count(specs)?),
            count: n,
            sample_rate: rate as f32,
            ui: ui::UiState::new(),
            random_seed: 0x9e37_79b9 ^ rate ^ n as u32,
            trigger_high: [false; 8],
        })
    }

    fn create_voice(_: &[i32], _: usize) -> Option<dsp::Block> {
        Some(dsp::Block::ZERO)
    }

    fn metadata(specs: &[i32], metadata: &mut Metadata<'_>) -> Option<()> {
        let n = count(specs)?;
        for (i, d) in [
            definition(c"Output", 1, LAST_BUS as i16, 13, UNIT_AUDIO_OUTPUT),
            definition(c"Output mode", 0, 1, 0, UNIT_OUTPUT_MODE),
            definition(c"Pitch", 0, 127, 57, UNIT_MIDI_NOTE),
            definition(c"Fine tune", -100, 100, 0, UNIT_CENTS),
            definition(c"Pitch CV (1V/oct)", 0, LAST_BUS as i16, 0, UNIT_CV_INPUT),
            definition(c"Smoothing", 0, 100, 0, UNIT_PERCENT),
            definition(c"Level", 0, 100, 50, UNIT_PERCENT),
        ]
        .into_iter()
        .enumerate()
        {
            metadata.set_parameter(i, d?)?;
        }
        metadata.add_page(
            c"Oscillator",
            1,
            &[
                PITCH as u8,
                FINE as u8,
                SMOOTHING as u8,
                fold_limit_parameter(n) as u8,
                LEVEL as u8,
                randomize_parameter(n) as u8,
                randomize_boundaries_parameter(n) as u8,
                randomize_curves_parameter(n) as u8,
                zero_curves_parameter(n) as u8,
                shape_triggers(n)[4] as u8,
                shape_triggers(n)[5] as u8,
                shape_triggers(n)[6] as u8,
                shape_triggers(n)[7] as u8,
                centroid_parameter(n) as u8,
                strength_parameter(n) as u8,
            ],
        )?;
        metadata.add_page(
            c"Routing",
            2,
            &[OUTPUT as u8, OUTPUT_MODE as u8, PITCH_CV as u8],
        )?;
        for (i, name) in BOUNDARY_NAMES.iter().enumerate().take(n - 1) {
            metadata.set_parameter(
                boundary_parameter(i + 1),
                definition(name, -100, 100, dsp::default_level(i + 1, n), UNIT_PERCENT)?,
            )?;
        }
        for (i, name) in CURVE_NAMES.iter().enumerate().take(n) {
            metadata.set_parameter(
                curve_parameter(n, i),
                definition(name, -100, 100, 0, UNIT_PERCENT)?,
            )?;
        }
        // Both ends of the cycle reference the same editable seam boundary.
        metadata.set_parameter(
            seam_parameter(n),
            definition(c"Boundary 00", -100, 100, 0, UNIT_PERCENT)?,
        )?;
        for (parameter, name) in shape_triggers(n).into_iter().zip([
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
            centroid_parameter(n),
            definition(
                c"Warp Position",
                0,
                dsp::CENTROID_MAX,
                dsp::CENTROID_DEFAULT,
                UNIT_NONE,
            )?,
        )?;
        metadata.set_parameter(
            strength_parameter(n),
            definition(c"Warp Amount", -100, 100, 0, UNIT_PERCENT)?,
        )?;
        metadata.set_parameter(
            fold_limit_parameter(n),
            definition(
                c"Fold Limit",
                0,
                dsp::MAX_FOLDS,
                dsp::DEFAULT_FOLD_LIMIT,
                UNIT_NONE,
            )?,
        )?;
        // Keep the page count fixed, including at the maximum segment count.
        // Walk the shape in time order, repeating Boundary 00 at the end.
        let mut indices = [0u8; dsp::MAX_SEGMENTS * 2 + 1];
        *indices.first_mut()? = seam_parameter(n) as u8;
        for segment in 0..n {
            *indices.get_mut(segment * 2 + 1)? = curve_parameter(n, segment) as u8;
            *indices.get_mut(segment * 2 + 2)? =
                editor_parameter(n, segment, 1).unwrap_or(seam_parameter(n)) as u8;
        }
        metadata.add_page(c"Waveform", 3, indices.get(..n * 2 + 1)?)?;
        Some(())
    }

    fn parameter_changed(context: &mut Callback<'_, Audio>, parameter: i32) {
        let n = context.state().count;
        let triggers = shape_triggers(n);
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
                .set(seam_parameter(n), dsp::preset_boundary(shape, 0, n));
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
                .set(seam_parameter(n), random_level(&mut seed));
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
                let _ = context.parameters().set(curve_parameter(n, segment), value);
            }
        }
        context.state_mut().random_seed = seed;
    }

    fn step(context: &mut Callback<'_, Audio>, samples: &mut [f32], frames: usize) {
        let n = context.state().count.clamp(2, 32);
        let p = context.read_parameters();
        let wave = read_wave(n, |index| p.value(index));
        let output = p.value(OUTPUT).unwrap_or(13);
        let replace = p.value(OUTPUT_MODE).unwrap_or(0) != 0;
        let note = p.value(PITCH).unwrap_or(57).clamp(0, 127) as f32;
        let fine = p.value(FINE).unwrap_or(0).clamp(-100, 100) as f32 / 1200.0;
        let cv_bus = p.value(PITCH_CV).unwrap_or(0);
        let level = p.value(LEVEL).unwrap_or(50).clamp(0, 100) as f32 * 0.01;
        let Some(mut buses) = AudioBuses::new(samples, frames) else {
            return;
        };
        let cv = buses.read_first(cv_bus);
        let cv = if cv.is_finite() {
            cv.clamp(-10.0, 10.0)
        } else {
            0.0
        };
        let frequency = 440.0
            * distingnt_api::api::math::exp2(((note - 69.0) / 12.0 + fine + cv).clamp(-16.0, 16.0));
        let Some(output) = buses.bus_mut(output) else {
            return;
        };
        let processing = context.processing();
        let state = processing.state;
        let memory = processing.voices;
        state.engine.request(wave);
        state.engine.advance(memory);
        for sample in output {
            let value = state
                .engine
                .tick(memory, frequency, state.sample_rate, level);
            if replace {
                *sample = value;
            } else {
                *sample += value;
            }
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
