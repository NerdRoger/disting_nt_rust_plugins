//! Safe framework-based synth. DSP state retains its original DTC placement.
#![no_std]
#![cfg_attr(not(test), forbid(unsafe_code))]
#![cfg_attr(test, deny(unsafe_code))]
use distingnt_api::api::{
    audio::AudioBuses,
    metadata::{Definition, EnumStrings},
    parameters::Audio,
    plugin::{Configuration, Context, Metadata, Plugin, Specification},
    state_memory::Dtc,
};
use distingnt_api::*;

const PARAM_OUTPUT: usize = 0;
const PARAM_OUTPUT_MODE: usize = 1;
const PARAM_MIDI_CHANNEL: usize = 2;
const PARAM_WAVEFORM: usize = 3;
const PARAMETER_COUNT: usize = 4;

#[repr(C)]
#[derive(Clone, Copy)]
struct SynthState {
    phase: u32,
    increment: u32,
    envelope: f32,
    gate: bool,
}

impl SynthState {
    const fn new() -> Self {
        Self {
            phase: 0,
            increment: 0,
            envelope: 0.0,
            gate: false,
        }
    }
    fn note_off(&mut self) {
        self.gate = false;
    }
    fn note_on(&mut self, increment: u32) {
        self.increment = increment;
        self.gate = true;
    }
    fn render(&mut self, parameters: Parameters, output: &mut [f32]) {
        let target = if self.gate { 1.0 } else { 0.0 };
        for sample in output {
            self.phase = self.phase.wrapping_add(self.increment);
            let wave = if parameters.sawtooth {
                (self.phase as i32 as f32) * (5.0 / 2_147_483_648.0)
            } else if (self.phase as i32) < 0 {
                -5.0
            } else {
                5.0
            };
            self.envelope = target + 0.99 * (self.envelope - target);
            if parameters.replace_output {
                *sample = wave * self.envelope;
            } else {
                *sample += wave * self.envelope;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Parameters {
    output_bus: usize,
    replace_output: bool,
    midi_channel: u8,
    sawtooth: bool,
}
impl Parameters {
    fn read(value: impl Fn(usize) -> Option<i16>) -> Option<Self> {
        Some(Self {
            output_bus: value(PARAM_OUTPUT)?.clamp(1, LAST_BUS as i16) as usize - 1,
            replace_output: value(PARAM_OUTPUT_MODE)? != 0,
            midi_channel: value(PARAM_MIDI_CHANNEL)?.clamp(1, 16) as u8,
            sawtooth: value(PARAM_WAVEFORM)? != 0,
        })
    }
}
static WAVEFORMS: EnumStrings<2> = EnumStrings::new([c"Square", c"Sawtooth"]);
const fn parameter(
    name: &'static core::ffi::CStr,
    min: i16,
    max: i16,
    default: i16,
    unit: u8,
) -> Definition {
    match Definition::new(name, min, max, default, unit, SCALING_NONE, None) {
        Some(value) => value,
        None => panic!("invalid synth definition"),
    }
}
static DEFINITIONS: [Definition; 4] = [
    parameter(c"Output", 1, LAST_BUS as i16, 13, UNIT_AUDIO_OUTPUT),
    parameter(c"Output mode", 0, 1, 0, UNIT_OUTPUT_MODE),
    parameter(c"MIDI channel", 1, 16, 1, UNIT_NONE),
    match Definition::new(
        c"Waveform",
        0,
        1,
        0,
        UNIT_ENUM,
        SCALING_NONE,
        Some(WAVEFORMS.table()),
    ) {
        Some(value) => value,
        None => panic!("invalid waveform definition"),
    },
];
struct RustSynth;
type Callback<'a> = Context<'a, SynthState, (), Audio, Dtc>;
impl Plugin for RustSynth {
    type State = SynthState;
    type Voice = ();
    type StateMemory = Dtc;
    const GUID: u32 = multichar(b'A', b'T', b'm', b's');
    const NAME: &'static core::ffi::CStr = c"Rustsynth";
    const DESCRIPTION: &'static core::ffi::CStr = c"A Rust MIDI synth with a safe DSP core";
    const TAGS: u32 = 0;
    const SPECIFICATIONS: &'static [Specification] = &[];
    const MIDI: bool = true;
    fn configuration(_: &[i32]) -> Option<Configuration> {
        Some(Configuration {
            parameters: PARAMETER_COUNT,
            pages: 2,
            page_indices: 4,
            voices: 0,
        })
    }
    fn create(_: &[i32], _: u32) -> Option<SynthState> {
        Some(SynthState::new())
    }
    fn create_voice(_: &[i32], _: usize) -> Option<()> {
        Some(())
    }
    fn metadata(_: &[i32], metadata: &mut Metadata<'_>) -> Option<()> {
        for (index, definition) in DEFINITIONS.iter().copied().enumerate() {
            metadata.set_parameter(index, definition)?;
        }
        metadata.add_page(
            c"Setup",
            0,
            &[PARAM_MIDI_CHANNEL as u8, PARAM_WAVEFORM as u8],
        )?;
        metadata.add_page(
            c"Routing",
            0,
            &[PARAM_OUTPUT as u8, PARAM_OUTPUT_MODE as u8],
        )
    }
    fn midi_message(context: &mut Callback<'_>, byte0: u8, byte1: u8, byte2: u8) {
        let Some(parameters) = Parameters::read(|index| context.read_parameters().value(index))
        else {
            return;
        };
        let status = byte0 & 0xf0;
        if (status != 0x80 && status != 0x90) || (byte0 & 0x0f) != parameters.midi_channel - 1 {
            return;
        }
        if status == 0x80 || byte2 == 0 {
            context.state_mut().note_off();
        } else {
            let semitones = (byte1 as f32 - 69.0) / 12.0;
            let increment = (distingnt_api::api::math::exp2(semitones) * 440.0 * 4_294_967_296.0
                / distingnt_api::api::sample_rate() as f32) as u32;
            context.state_mut().note_on(increment);
        }
    }
    fn step(context: &mut Callback<'_>, samples: &mut [f32], frames: usize) {
        let Some(parameters) = Parameters::read(|index| context.read_parameters().value(index))
        else {
            return;
        };
        let Some(mut buses) = AudioBuses::new(samples, frames) else {
            return;
        };
        let Some(output) = buses.bus_mut((parameters.output_bus + 1) as i16) else {
            return;
        };
        context.state_mut().render(parameters, output);
    }
}
distingnt_api::export_plugin!(RustSynth);

#[cfg(test)]
#[allow(unsafe_code)]
#[path = "tests.rs"]
mod tests;
