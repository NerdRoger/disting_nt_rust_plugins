use core::f32::consts::LOG2_10;

use distingnt_api::api::math::exp2;

use super::{
    mix::db_gain,
    punch::{ResonantWires, Rim, TwoStageBody},
    wave_math::{one_pole_coefficient as pole, positive_pow, sine, wrap, DampedMode},
};
use crate::{
    lookup::{self, Lookup},
    LayerPatch, Model, CONTROLS,
};
pub(super) use crate::random::signed_noise as noise;

fn decay_exponent(ms: f32, rate: u32) -> f32 {
    (-2000.0 * LOG2_10) / (ms * rate as f32)
}

pub(super) fn decay_step(ms: f32, rate: u32) -> f32 {
    exp2(decay_exponent(ms, rate))
}

pub(super) fn decay_at(age_frames: f32, ms: f32, rate: u32) -> f32 {
    exp2((-2000.0 * LOG2_10 * age_frames) / (ms * rate as f32))
}

pub(super) fn advance(value: &mut f32, step: f32) -> f32 {
    let old = *value;
    *value *= step;
    old
}

#[derive(Clone, Copy)]
pub(super) struct Band {
    low: f32,
    high: f32,
    low_step: f32,
    high_step: f32,
}

impl Band {
    pub(super) fn new(low: f32, high: f32, rate: u32) -> Self {
        Self {
            low: 0.0,
            high: 0.0,
            low_step: pole(low, rate),
            high_step: pole(high, rate),
        }
    }

    pub(super) fn sample(&mut self, sample: f32) -> f32 {
        self.low += self.low_step * (sample - self.low);
        self.high += self.high_step * (sample - self.high);
        2.0 * (self.high - self.low)
    }
}

#[derive(Clone, Copy)]
struct PitchBody {
    phase: f32,
    amplitude: f32,
    decay: f32,
}

#[derive(Clone, Copy)]
struct Sweep {
    base: f32,
    offset: f32,
    decay: f32,
}

impl Sweep {
    fn new(base: f32, semitones: f32, ms: f32, rate: u32) -> Self {
        Self {
            base,
            offset: semitones / 12.0,
            decay: decay_step(ms, rate),
        }
    }

    fn frequency(&mut self, tables: Option<Lookup<'_>>) -> f32 {
        let octaves = advance_octaves(&mut self.offset, self.decay);
        self.at(octaves, tables)
    }

    fn at(&self, octaves: f32, tables: Option<Lookup<'_>>) -> f32 {
        if octaves <= 0.000001 {
            self.base
        } else {
            self.base * lookup::pitch_multiplier(tables, octaves)
        }
    }
}

fn advance_octaves(octaves: &mut f32, step: f32) -> f32 {
    if *octaves <= 0.000001 {
        0.0
    } else {
        advance(octaves, step)
    }
}

#[derive(Clone, Copy)]
enum Engine {
    Pitch {
        body: PitchBody,
        sweep: Sweep,
        curve: f32,
        curve_width: f32,
        click: Band,
        click_level: f32,
        click_decay: f32,
    },
    Resonator {
        sweep: Sweep,
        drift: Sweep,
        real: f32,
        imaginary: f32,
        filtered: f32,
        tone: f32,
        damping: f32,
        pulse_step: f32,
        pulse_gain: f32,
    },
    Modal([DampedMode; 5]),
    Wires(Band),
    SweepWires {
        sweep: Sweep,
        focus: f32,
        grain: f32,
        low: f32,
        band: f32,
        steady: WireCoefficients,
    },
    Rim(Rim),
    TwoStage(TwoStageBody),
    ResonantWires(ResonantWires),
}

#[derive(Clone, Copy)]
struct WireCoefficients {
    g: f32,
    response: f32,
    compensation: f32,
}

impl WireCoefficients {
    fn new(hz: f32, focus: f32, rate: u32) -> Self {
        let turns = 0.5 * hz / rate as f32;
        let g = sine(turns) / sine(turns + 0.25);
        Self {
            g,
            response: (1.0 + g * (g + focus.recip())).recip(),
            compensation: 0.55 * positive_pow(rate as f32 / (hz * focus), 0.5),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Voice {
    engine: Engine,
    seed: u32,
    rate: u32,
    delay: f32,
    hold: usize,
    ms_per_frame: f32,
    wire_amp: f32,
    wire_step: f32,
    attack: f32,
    boost: f32,
    blend: f32,
    release: f32,
    length: usize,
    pub(super) position: usize,
}

fn damped_mode(hz: f32, level: f32, ms: f32, rate: u32) -> DampedMode {
    DampedMode::new((hz / rate as f32, decay_exponent(ms, rate), level))
}

fn modal_tones(p: &[f32; CONTROLS], rate: u32) -> [DampedMode; 5] {
    let position = (p[4] - 50.0) / 50.0;
    let mode = |ratio, level, length, edge| {
        damped_mode(
            p[0] * ratio,
            0.8 * level * (1.0 + position * edge),
            length,
            rate,
        )
    };
    let upper = |ratio, level, length, edge| {
        mode(
            ratio,
            level * p[2] / 100.0,
            p[1] * length / (0.5 + p[3] / 55.0),
            edge,
        )
    };
    [
        mode(1.0, 0.86, p[1], -0.55),
        upper(1.59, 0.57, 0.66, 0.7),
        upper(2.14, 0.37, 0.45, -0.4),
        upper(2.92, 0.21, 0.25, 0.9),
        upper(3.60, 0.12, 0.17, 0.8),
    ]
}

impl Voice {
    pub(super) fn new(
        patch: LayerPatch,
        layer: usize,
        rate: u32,
        length: usize,
        truncated: bool,
        tables: Option<Lookup<'_>>,
    ) -> Option<Self> {
        let p = patch.controls;
        let mut delay = 0.0;
        let mut wire_amp = 1.0;
        let mut wire_step = 1.0;
        let mut attack = (rate as f32 * 0.0007).recip();
        let ms_per_frame = 1000.0 / rate as f32;
        let hold_ms = match patch.model {
            Model::PitchSweep | Model::Resonator => p[9],
            Model::Modal => p[6],
            _ => 0.0,
        };
        let hold = (rate as usize * hold_ms as usize).div_ceil(1000);
        let (engine, drive) = match patch.model {
            Model::Off => return None,
            Model::PitchSweep => (
                Engine::Pitch {
                    body: PitchBody {
                        phase: 0.0,
                        amplitude: 0.7,
                        decay: decay_step(p[1], rate),
                    },
                    sweep: Sweep::new(p[0] / rate as f32, p[2], p[3], rate),
                    curve: exp2(p[4] / 50.0),
                    curve_width: rate as f32 * p[3] * 0.001,
                    click: Band::new(900.0, p[6], rate),
                    click_level: p[5] * 0.01,
                    click_decay: if p[5] > 0.0 {
                        decay_step(p[7], rate)
                    } else {
                        0.0
                    },
                },
                p[8],
            ),
            Model::Resonator => {
                attack = 0.0;
                let width = p[6] * 0.001;
                (
                    Engine::Resonator {
                        sweep: Sweep::new(p[0], p[2], p[3], rate),
                        drift: Sweep::new((rate as f32).recip(), p[4], 90.0, rate),
                        real: 0.0,
                        imaginary: 0.0,
                        filtered: 0.0,
                        tone: pole(p[7], rate),
                        damping: decay_step(p[1], rate),
                        pulse_step: (width * rate as f32).recip(),
                        pulse_gain: 0.01 * p[5] * (0.0005 / width).min(1.0),
                    },
                    p[8],
                )
            }
            Model::Modal => (Engine::Modal(modal_tones(&p, rate)), p[5]),
            Model::Wires | Model::SweepWires => {
                delay = ((p[2] * 10.0 + 0.5) as u32 * rate) as f32 / 10_000.0;
                let first_age = 1.0 + delay as u32 as f32 - delay;
                wire_amp = decay_at(first_age, p[1], rate);
                wire_step = decay_step(p[1], rate);
                let rise = if patch.model == Model::Wires {
                    p[4]
                } else {
                    p[6]
                };
                attack = ms_per_frame / rise;
                if patch.model == Model::Wires {
                    (Engine::Wires(Band::new(p[0] / 3.0, p[0] * 2.0, rate)), p[3])
                } else {
                    let mut sweep = Sweep::new(p[0], 7.0, 62.0, rate);
                    sweep.offset *= decay_at(first_age, 62.0, rate);
                    (
                        Engine::SweepWires {
                            sweep,
                            focus: p[3],
                            grain: 0.00425 * p[4],
                            low: 0.0,
                            band: 0.0,
                            steady: WireCoefficients::new(p[0], p[3], rate),
                        },
                        p[5],
                    )
                }
            }
            Model::Rim => {
                attack = 0.0;
                (Engine::Rim(Rim::new(&p, rate)), p[4])
            }
            Model::TwoStage => {
                attack = 0.0;
                (Engine::TwoStage(TwoStageBody::new(&p, rate, tables)), 0.0)
            }
            Model::ResonantWires => {
                attack = 0.0;
                (Engine::ResonantWires(ResonantWires::new(&p, rate)), 0.0)
            }
        };
        Some(Self {
            engine,
            seed: (layer as u32 + 1).wrapping_mul(0x56d62e11),
            rate,
            delay,
            hold,
            ms_per_frame,
            wire_amp,
            wire_step,
            attack,
            boost: db_gain(drive),
            blend: (drive / 5.0).min(1.0),
            release: if truncated || matches!(patch.model, Model::TwoStage | Model::ResonantWires) {
                0.0
            } else {
                (rate as f32 * 0.005).recip()
            },
            length,
            position: 0,
        })
    }

    pub(super) fn render(&mut self, output: &mut [f32], tables: Option<Lookup<'_>>) {
        // Each block starts from the exact phase and envelope to bound drift
        // in the quadrature oscillators over the duration of the hit.
        if let Engine::Modal(modes) = &mut self.engine {
            for mode in modes {
                mode.anchor_with_hold(self.position, self.hold);
            }
        }
        for sample in output {
            *sample = self.sample(tables);
            self.position += 1;
        }
    }

    fn sample(&mut self, tables: Option<Lookup<'_>>) -> f32 {
        let frame = self.position as f32;
        let age = frame - self.delay;
        let mut raw = match &mut self.engine {
            Engine::Pitch {
                body,
                sweep,
                curve,
                curve_width,
                click,
                click_level,
                click_decay,
            } => {
                let body_sample = sine(body.phase) * body.amplitude;
                if self.position >= self.hold {
                    body.amplitude *= body.decay;
                }
                let frequency = if *curve == 1.0 {
                    sweep.frequency(tables)
                } else if sweep.offset == 0.0 {
                    sweep.base
                } else {
                    let warped = positive_pow(frame / *curve_width, *curve);
                    let octaves = sweep.offset * exp2(-2.0 * LOG2_10 * warped);
                    if octaves <= 0.000001 {
                        sweep.offset = 0.0;
                    }
                    sweep.at(octaves, tables)
                };
                body.phase = wrap(body.phase + frequency);
                let hit = if *click_level > 0.000001 {
                    click.sample(noise(&mut self.seed)) * advance(click_level, *click_decay)
                } else {
                    0.0
                };
                body_sample + hit
            }
            Engine::Resonator {
                sweep,
                drift,
                real,
                imaginary,
                filtered,
                tone,
                damping,
                pulse_step,
                pulse_gain,
            } => {
                let turns = sweep.frequency(tables) * drift.frequency(tables);
                let cosine = sine(turns + 0.25);
                let sine_value = sine(turns);
                let pulse_phase = frame * *pulse_step;
                let pulse = if pulse_phase < 1.0 {
                    sine(pulse_phase) * *pulse_gain
                } else {
                    0.0
                };
                let previous = *real;
                let loss = if self.position < self.hold {
                    1.0
                } else {
                    *damping
                };
                *real = loss * (*real * cosine - *imaginary * sine_value);
                *imaginary = loss * (*imaginary * cosine + previous * sine_value + pulse);
                *filtered += *tone * (*real * 1.5 + pulse * 0.32 - *filtered);
                0.3 * *filtered
            }
            Engine::Modal(modes) => {
                if self.hold != 0 && self.position == self.hold {
                    for mode in modes.iter_mut() {
                        mode.anchor_with_hold(self.position, self.hold);
                    }
                }
                modes.iter_mut().map(DampedMode::next).sum()
            }
            Engine::Wires(filter) => {
                // The app warms this filter during the delay.
                let filtered = filter.sample(noise(&mut self.seed));
                if age <= 0.0 {
                    return 0.0;
                }
                filtered * advance(&mut self.wire_amp, self.wire_step)
            }
            Engine::SweepWires {
                sweep,
                focus,
                grain,
                low,
                band,
                steady,
            } => {
                // Advance the noise stream during the delay while leaving the SVF at rest.
                let input = noise(&mut self.seed);
                if age <= 0.0 {
                    return 0.0;
                }
                let octaves = advance_octaves(&mut sweep.offset, sweep.decay);
                let coeff = if octaves > 0.0 {
                    let q = (*focus
                        * lookup::pitch_multiplier(tables, octaves * (3.0 / 7.0)).recip())
                    .clamp(0.35, 3.5);
                    WireCoefficients::new(sweep.at(octaves, tables), q, self.rate)
                } else {
                    *steady
                };
                let y = (*band + coeff.g * (input - *low)) * coeff.response;
                let new_low = *low + coeff.g * y;
                *band = 2.0 * y - *band;
                *low = 2.0 * new_low - *low;
                let texture = 1.0 + *grain * sine(73.0 * age / self.rate as f32);
                y * coeff.compensation * texture * advance(&mut self.wire_amp, self.wire_step)
            }
            Engine::Rim(rim) => rim.sample(frame * self.ms_per_frame, &mut self.seed),
            Engine::TwoStage(body) => body.sample(frame * self.ms_per_frame, tables),
            Engine::ResonantWires(wires) => wires.sample(frame * self.ms_per_frame),
        };
        if self.attack > 0.0 {
            raw *= (age * self.attack).min(1.0);
        }
        if self.blend > 0.0 {
            raw += (lookup::drive(tables, raw * self.boost) - raw) * self.blend;
        }
        if self.release > 0.0 {
            raw *=
                (((self.length - 1).saturating_sub(self.position)) as f32 * self.release).min(1.0);
        }
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::natural_frames;

    #[test]
    fn each_voice_matches_samples_from_the_web_app_at_48_khz() {
        // Captured from SnareLab.html renderVoice with the default patch and layer ID 2.
        let frames = [1, 14, 37, 143, 144, 145, 222, 511, 1600, 5600];
        let reference = [
            (
                Model::PitchSweep,
                [
                    0.000726389,
                    0.134089693,
                    0.633682370,
                    -0.597905636,
                    -0.601189911,
                    -0.603976786,
                    0.251960784,
                    0.430754453,
                    -0.176620483,
                    -0.000416055,
                ],
            ),
            (
                Model::Resonator,
                [
                    0.005076766,
                    -0.426212996,
                    -0.613594770,
                    0.542048335,
                    0.525421262,
                    0.508332849,
                    -0.737761140,
                    0.080632068,
                    -0.322289497,
                    -0.023610516,
                ],
            ),
            (
                Model::Modal,
                [
                    0.001509085,
                    0.264377445,
                    0.956920087,
                    -0.285639107,
                    -0.291167378,
                    -0.296469688,
                    -0.408594638,
                    0.066000678,
                    0.199978992,
                    -0.010806820,
                ],
            ),
            (
                Model::Wires,
                [
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    -0.006705012,
                    -0.529935300,
                    0.607402563,
                    0.138893083,
                    -0.135413378,
                ],
            ),
            (
                Model::SweepWires,
                [
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.002171963,
                    -0.252529025,
                    0.604236007,
                    0.294313133,
                    -0.184807628,
                ],
            ),
            (
                Model::Rim,
                [
                    -0.012082675,
                    -0.004425152,
                    -0.557994962,
                    0.039810307,
                    -0.053900748,
                    0.239498973,
                    -0.162283525,
                    0.082419768,
                    0.003769844,
                    0.0,
                ],
            ),
        ];
        for (model, expected) in reference {
            let mut patch = LayerPatch {
                model,
                ..LayerPatch::OFF
            };
            for (slot, control) in patch.controls.iter_mut().zip(model.controls()) {
                *slot = control.decode(control.initial);
            }
            if model == Model::Rim {
                patch
                    .controls
                    .copy_from_slice(&[1060.0, 36.0, 9000.0, 14.0, 2.0, 0.0, 0.0, 0.0, 0.0, 100.0]);
            }
            let rate = 48_000;
            let length = natural_frames(&patch, rate);
            let mut voice = Voice::new(patch, 1, rate, length, false, None).unwrap();
            let mut actual = [0.0; 5601];
            for chunk in actual[..length.min(5601)].chunks_mut(256) {
                voice.render(chunk, None);
            }
            for (frame, expected) in frames.into_iter().zip(expected) {
                let got = actual[frame];
                assert!(
                    (got - expected).abs() < 0.0025,
                    "model {} frame {frame}: {got} vs {expected}",
                    model as u8
                );
            }
        }
    }
}
