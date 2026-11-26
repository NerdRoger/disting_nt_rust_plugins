#[path = "../../layered-drums/mix.rs"]
mod mix;
#[path = "punch.rs"]
mod punch;
#[path = "voice.rs"]
mod voice;
#[path = "../../layered-drums/wave_math.rs"]
mod wave_math;

use distingnt_api::api::cpu_cycle_count;

use self::{
    mix::{db_gain, mix_pair, mix_single, mix_tile, MasterDrive, MixInput, TailTaper},
    punch::{body_duration, rim_duration, shape_punch, wires_duration},
    voice::Voice,
};
use crate::{layer_slot, lookup::Lookup, Job, Layer, LayerPatch, Model, Scope, WaveBuffers, MAX_LAYERS};

const MAX_MS: u32 = 750;
const TAPER_MS: u32 = 650;
const SYNTHESIS_FRAMES: usize = 256;
const MASTER_FRAMES: usize = 512;
const SCOPE_COLUMNS: usize = 8;
const SLICE_CYCLES: u32 = 1_800_000;
const SLICE_UNITS: usize = 32;

pub(super) fn max_frames(rate: u32) -> usize {
    (rate as usize * MAX_MS as usize).div_ceil(1000)
}

fn natural_frames(patch: &LayerPatch, rate: u32) -> usize {
    let p = &patch.controls;
    // The two new fitted models use rounded durations in the browser.
    let rounded_ms = match patch.model {
        Model::TwoStage => Some(body_duration(p)),
        Model::ResonantWires => Some(wires_duration(p)),
        _ => None,
    };
    if let Some(duration) = rounded_ms {
        return (duration * rate as f32 / 1000.0 + 0.5) as usize;
    }
    if patch.model == Model::Rim {
        let samples = rim_duration(p) * rate as f32 / 1000.0;
        let whole = samples as usize;
        return whole + usize::from(samples > whole as f32);
    }
    let (body_hold, body_decay, click_decay, wire_delay) = match patch.model {
        Model::Off => return 0,
        Model::PitchSweep => (p[9], p[1], if p[5] > 0.0 { p[7] } else { 0.0 }, 0.0),
        Model::Resonator => (p[9], p[1], 0.0, 0.0),
        Model::Modal => (p[6], p[1], 0.0, 0.0),
        Model::Wires | Model::SweepWires => (0.0, p[1], 0.0, p[2]),
        _ => return 0,
    };
    let tenths = |ms: f32| (ms * 10.0 + 0.5) as usize;
    let body = tenths(body_hold + wire_delay) * 100 + tenths(body_decay) * 150;
    let thousandths_of_ms = body.max(tenths(click_decay) * 150) + 5000;
    ((thousandths_of_ms as u64 * u64::from(rate)).div_ceil(1_000_000)) as usize
}

#[derive(Clone, Copy)]
pub(super) struct CachedLayer(Option<LayerPatch>);

impl CachedLayer {
    pub(super) const EMPTY: Self = Self(None);
}

pub(super) enum RenderStatus {
    Pending,
    Complete(usize),
}

#[derive(Clone, Copy)]
enum Stage {
    Synthesis,
    Mix,
    Drive,
    Scope,
}

#[derive(Clone, Copy)]
pub(super) struct RenderPlan {
    stage: Stage,
    position: usize,
    layer: usize,
    voice: Option<Voice>,
    inputs: [Option<MixInput>; MAX_LAYERS],
    count: usize,
    punch: [f32; MAX_LAYERS],
    has_punch: bool,
    length: usize,
    taper: Option<TailTaper>,
    gain: f32,
    boost: Option<f32>,
    peak: f32,
    display_peak: f32,
}

impl RenderPlan {
    pub(super) fn new(job: &Job, capacity: usize) -> Self {
        let solo = Layer::solo_active(&job.layers);
        let mut inputs = [None; MAX_LAYERS];
        let mut punch = [0.0; MAX_LAYERS];
        let mut has_punch = false;
        let mut natural_length = 0;
        let mut count = 0;
        for (index, ((layer, input), amount)) in job
            .layers
            .iter()
            .zip(&mut inputs)
            .zip(&mut punch)
            .enumerate()
        {
            if layer.audible(solo) {
                *amount = match layer.patch.model {
                    Model::TwoStage => 1.0,
                    Model::Rim => layer.patch.controls[5] * 0.01,
                    _ => 0.0,
                };
                has_punch |= *amount > 0.0;
                let natural = natural_frames(&layer.patch, job.rate);
                natural_length = natural_length.max(natural);
                *input = Some(MixInput {
                    slot: layer_slot(usize::from(job.output_buffers), index),
                    end: natural.min(capacity),
                    level: layer.level,
                });
                count += 1;
            }
        }
        let length = natural_length.min(capacity);
        Self {
            stage: if count == 0 {
                Stage::Scope
            } else {
                Stage::Synthesis
            },
            position: 0,
            layer: 0,
            voice: None,
            inputs,
            count,
            punch,
            has_punch,
            length,
            taper: (natural_length > capacity).then(|| tail_taper(job.rate, length)),
            gain: db_gain(job.gain),
            boost: (job.drive > 0.0).then(|| db_gain(job.drive)),
            peak: 0.0,
            display_peak: 0.0,
        }
    }

    fn enter(&mut self, stage: Stage) {
        self.position = 0;
        self.stage = stage;
    }

    pub(super) fn advance(
        &mut self,
        job: &Job,
        cached: &mut [CachedLayer; MAX_LAYERS],
        buffers: &WaveBuffers,
        samples: &mut [f32],
        tile: &mut [f32],
        punch_tile: &mut [f32],
        tables: Option<Lookup<'_>>,
        scope: &mut Scope,
        start: u32,
    ) -> Option<RenderStatus> {
        for _ in 0..SLICE_UNITS {
            match self.stage {
                Stage::Synthesis => self.synthesize(job, cached, buffers, tables)?,
                Stage::Mix => {
                    self.mix(buffers, samples, tile, punch_tile, job.rate)?;
                    if self.position == self.length {
                        self.enter(if self.boost.is_some() {
                            Stage::Drive
                        } else {
                            Stage::Scope
                        });
                    }
                }
                Stage::Drive => {
                    let end = (self.position + MASTER_FRAMES).min(self.length);
                    MasterDrive::new(self.boost?, self.peak, self.gain)
                        .apply(samples.get_mut(self.position..end)?);
                    self.position = end;
                    if end == self.length {
                        self.peak = 1.0;
                        self.enter(Stage::Scope);
                    }
                }
                Stage::Scope => {
                    let end = (self.position + SCOPE_COLUMNS).min(Scope::COLUMNS);
                    let range = self.position..end;
                    let trim = if self.peak > 1.0 {
                        self.peak.recip()
                    } else {
                        1.0
                    };
                    let mix = samples.get_mut(..self.length)?;
                    let peak = if let Some(taper) = self.taper {
                        scope.capture_columns(
                            mix,
                            max_frames(job.rate) as u32,
                            range,
                            |sample, frame| {
                                *sample *= trim * taper.gain(frame);
                            },
                        )
                    } else if trim < 1.0 {
                        scope.capture_columns(
                            mix,
                            max_frames(job.rate) as u32,
                            range,
                            |sample, _| *sample *= trim,
                        )
                    } else {
                        scope.capture_columns(mix, max_frames(job.rate) as u32, range, |_, _| {})
                    };
                    self.display_peak = self.display_peak.max(peak);
                    self.position = end;
                    if end == Scope::COLUMNS {
                        scope.finish_capture(self.display_peak);
                        return Some(RenderStatus::Complete(self.length));
                    }
                }
            }
            if cpu_cycle_count().wrapping_sub(start) >= SLICE_CYCLES {
                break;
            }
        }
        Some(RenderStatus::Pending)
    }

    fn synthesize(
        &mut self,
        job: &Job,
        cached: &mut [CachedLayer; MAX_LAYERS],
        buffers: &WaveBuffers,
        tables: Option<Lookup<'_>>,
    ) -> Option<()> {
        while let Some(input) = self.inputs.get(self.layer) {
            let patch = &job.layers.get(self.layer)?.patch;
            let key = &mut cached.get_mut(self.layer)?.0;
            if let Some(input) = input.filter(|_| key.as_ref() != Some(patch)) {
                if self.voice.is_none() {
                    let truncated = natural_frames(patch, job.rate) > input.end;
                    self.voice =
                        Voice::new(*patch, self.layer, job.rate, input.end, truncated, tables);
                    *key = None;
                }
                let voice = self.voice.as_mut()?;
                let end = (voice.position + SYNTHESIS_FRAMES).min(input.end);
                buffers.with(input.slot, |wave| {
                    voice.render(wave.get_mut(voice.position..end)?, tables);
                    Some(())
                })??;
                if end == input.end {
                    *key = Some(*patch);
                    self.voice = None;
                    self.layer += 1;
                }
                return Some(());
            }
            self.layer += 1;
        }
        self.enter(Stage::Mix);
        Some(())
    }

    fn mix(
        &mut self,
        buffers: &WaveBuffers,
        samples: &mut [f32],
        tile: &mut [f32],
        punch_tile: &mut [f32],
        rate: u32,
    ) -> Option<()> {
        let start = self.position;
        let end = (start + MASTER_FRAMES).min(self.length);
        let dest = samples.get_mut(start..end)?;
        if self.has_punch {
            let clean = tile.get_mut(..dest.len())?;
            let punch = punch_tile.get_mut(..dest.len())?;
            clean.fill(0.0);
            punch.fill(0.0);
            for (input, &amount) in self.inputs.iter().zip(&self.punch) {
                let Some(input) = input else {
                    continue;
                };
                let frames = input.end.saturating_sub(start).min(dest.len());
                if frames == 0 {
                    continue;
                }
                buffers.with(input.slot, |wave| {
                    let wave = wave.get(start..start + frames)?;
                    if amount < 1.0 {
                        mix_single(
                            clean.get_mut(..frames)?,
                            wave,
                            frames,
                            input.level * (1.0 - amount),
                        )?;
                    }
                    if amount > 0.0 {
                        mix_single(punch.get_mut(..frames)?, wave, frames, input.level * amount)?;
                    }
                    Some(())
                })??;
            }
            let ms_per_frame = 1000.0 / rate as f32;
            for (offset, ((mixed, &clean), &punch)) in dest
                .iter_mut()
                .zip(clean.iter())
                .zip(punch.iter())
                .enumerate()
            {
                *mixed = clean + shape_punch(punch, (start + offset) as f32 * ms_per_frame);
            }
        } else if self.count >= 3 {
            mix_tile(buffers, dest, tile, start, &self.inputs)?;
        } else {
            let mut inputs = self.inputs.iter().flatten();
            let first = *inputs.next()?;
            if let Some(&second) = inputs.next() {
                mix_pair(buffers, dest, start, first, second)?;
            } else {
                buffers.with(first.slot, |wave| {
                    mix_single(dest, wave.get(start..end)?, 0, first.level)
                })??;
            }
        }
        if self.boost.is_some() {
            for sample in dest {
                self.peak = self.peak.max(sample.abs());
            }
        } else {
            for sample in dest {
                *sample *= self.gain;
                self.peak = self.peak.max(sample.abs());
            }
        }
        self.position = end;
        Some(())
    }
}

fn tail_taper(rate: u32, length: usize) -> TailTaper {
    let start = (rate as usize * TAPER_MS as usize).div_ceil(1000);
    TailTaper::new(start, length - 1)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::Model;

    #[test]
    fn wire_duration_and_taper_match_the_cap() {
        let rate = 48_000;
        let mut patch = LayerPatch {
            model: Model::Wires,
            ..LayerPatch::OFF
        };
        patch.controls[..3].copy_from_slice(&[3400.0, 500.0, 24.0]);
        assert_eq!(natural_frames(&patch, rate), 37_392);
        assert_eq!(max_frames(rate), 36_000);
        let taper = tail_taper(rate, max_frames(rate));
        assert_eq!(taper.gain(31_200), 1.0);
        assert!(taper.gain(33_599) > 0.7);
        assert_eq!(taper.gain(35_999), 0.0);
        patch.controls[1] = 260.0;
        patch.controls[2] = 3.0;
        assert_eq!(natural_frames(&patch, rate), 19_104);
    }

    #[test]
    fn new_punch_engines_match_the_web_reference_in_worker_blocks() {
        let frames = [
            48, 213, 367, 511, 890, 894, 900, 1345, 2231, 4000, 8001, 12345,
        ];
        let cases = [
            (
                Model::TwoStage,
                13763,
                [
                    0.0,
                    0.00363276,
                    -0.09840429,
                    -0.10109939,
                    -0.92611264,
                    -0.94446317,
                    -0.95978738,
                    -0.78632991,
                    0.55870945,
                    -0.00349606,
                    0.0,
                    0.0,
                ],
            ),
            (
                Model::ResonantWires,
                38400,
                [
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    -0.00001782,
                    0.00086108,
                    -0.03570103,
                    -0.13802216,
                    0.33108416,
                    -0.02182007,
                    0.03299579,
                ],
            ),
            (
                Model::Rim,
                12657,
                [
                    -0.25575637,
                    -0.3600878,
                    0.12637181,
                    0.03572421,
                    0.04466446,
                    0.07358034,
                    0.01306193,
                    0.04292931,
                    -0.01171832,
                    -0.00042304,
                    -0.00000571,
                    0.00000004,
                ],
            ),
        ];
        for (model, length, reference) in cases {
            let mut patch = LayerPatch {
                model,
                ..LayerPatch::OFF
            };
            for (value, control) in patch.controls.iter_mut().zip(model.controls()) {
                *value = control.decode(control.initial);
            }
            if model == Model::Rim {
                patch.controls.copy_from_slice(&[
                    1060.0, 36.0, 9000.0, 14.0, 2.0, 68.0, 87.0, 95.0, 75.0, 115.0,
                ]);
            }
            let rate = 48_000;
            assert_eq!(natural_frames(&patch, rate), length);
            let output_length = length.min(max_frames(rate));
            let mut voice =
                Voice::new(patch, 1, rate, output_length, length > output_length, None).unwrap();
            let mut samples = std::vec![0.0; output_length];
            for chunk in samples.chunks_mut(256) {
                voice.render(chunk, None);
            }
            for (frame, expected) in frames.into_iter().zip(reference) {
                let actual = samples[frame];
                assert!(
                    (actual - expected).abs() < 0.0025,
                    "model {} frame {frame}: {actual} vs {expected}",
                    model as u8
                );
            }
        }
    }
}
