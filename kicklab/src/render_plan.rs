use super::{
    cpu_cycle_count, db_gain, layer_frames, layer_slot, milliseconds, mix_pair, mix_single,
    mix_tile, render_cached_layer, CachedLayer, Job, Lookup, Lowpass, MasterDrive, MixInput, Model,
    RenderTiming, WaveBuffers, MAX_LAYERS, MIN_TILED_LAYERS, RELEASE_FADE_MS,
};
use crate::Scope;

// Roughly three milliseconds on the NT. Checks occur between complete units
// of work so each slice releases all borrowed DRAM spans before returning.
pub(crate) const SLICE_CYCLES: u32 = 1_800_000;
const MASTER_FRAMES: usize = 2048;
const SCOPE_COLUMNS: usize = 16;

#[derive(Clone, Copy)]
enum Stage {
    Layers,
    Mix,
    Filter,
    Peak,
    Drive,
    Fade,
    Scope,
}

pub enum RenderStatus {
    Pending,
    Complete(usize),
}

#[derive(Clone, Copy)]
pub struct RenderPlan {
    stage: Stage,
    position: usize,
    inputs: [Option<MixInput>; MAX_LAYERS],
    count: usize,
    dry_length: usize,
    length: usize,
    gain: f32,
    boost: Option<f32>,
    filter: Option<Lowpass>,
    fade: usize,
    peak: f32,
    display_peak: f32,
}

impl RenderPlan {
    pub fn new(job: &Job, max_frames: usize) -> Self {
        let solo = job
            .layers
            .iter()
            .any(|layer| layer.patch.model != Model::Off && layer.solo);
        let mut inputs = [None; MAX_LAYERS];
        let mut count = 0;
        let mut dry_length = 0;
        for (index, (layer, input)) in job.layers.iter().zip(&mut inputs).enumerate() {
            if layer.audible(solo) {
                let end = layer_frames(&layer.patch, job.rate).min(max_frames);
                *input = Some(MixInput {
                    slot: layer_slot(usize::from(job.output_buffers), index),
                    end,
                    level: layer.level,
                });
                count += 1;
                dry_length = dry_length.max(end);
            }
        }
        let cutoff = f32::from(job.cutoff).min(job.rate as f32 * 0.45);
        let (filter, length, fade) = if dry_length != 0 && cutoff > 0.0 {
            let tail = (job.rate as f32 * (3.0 / cutoff).max(0.005)) as usize + 1;
            let end = (dry_length + tail).min(max_frames);
            let fade = if end < dry_length + tail {
                milliseconds(job.rate, RELEASE_FADE_MS).min(end)
            } else {
                0
            };
            (Some(Lowpass::new(cutoff / job.rate as f32)), end, fade)
        } else {
            (None, dry_length, 0)
        };
        Self {
            stage: if dry_length == 0 {
                Stage::Scope
            } else {
                Stage::Layers
            },
            position: 0,
            inputs,
            count,
            dry_length,
            length,
            gain: db_gain(job.gain),
            boost: (job.drive > 0.0).then(|| db_gain(job.drive)),
            filter,
            fade,
            peak: 1.0,
            display_peak: 0.0,
        }
    }

    fn enter(&mut self, stage: Stage) {
        self.stage = stage;
        self.position = 0;
    }

    #[inline(never)]
    pub fn advance(
        &mut self,
        job: &Job,
        cached: &mut [CachedLayer; MAX_LAYERS],
        buffers: &WaveBuffers,
        samples: &mut [f32],
        tile: &mut [f32],
        tables: Option<Lookup<'_>>,
        timing: &mut RenderTiming,
        scope: &mut Scope,
        slice_start: u32,
    ) -> Option<RenderStatus> {
        let fused = self.count == 1 && self.filter.is_none() && self.boost.is_none();
        loop {
            let stage = self.stage;
            #[cfg(feature = "profiling")]
            let start = cpu_cycle_count();
            match stage {
                Stage::Layers => {
                    self.render_layer(job, cached, buffers, tables, timing)?;
                }
                Stage::Mix => {
                    self.mix(buffers, samples, tile, fused)?;
                    if self.position == self.dry_length {
                        if self.filter.is_some() {
                            self.peak = 0.0;
                            self.enter(Stage::Filter);
                        } else if self.boost.is_some() {
                            self.peak = 0.0;
                            self.enter(Stage::Peak);
                        } else if fused {
                            self.enter(Stage::Scope);
                        } else {
                            self.enter(Stage::Peak);
                        }
                    }
                }
                Stage::Filter => {
                    let end = (self.position + MASTER_FRAMES).min(self.length);
                    let span = samples.get_mut(self.position..end)?;
                    let tail = self
                        .dry_length
                        .saturating_sub(self.position)
                        .min(span.len());
                    span.get_mut(tail..)?.fill(0.0);
                    let gain = if self.boost.is_some() { 1.0 } else { self.gain };
                    self.peak = self.peak.max(self.filter.as_mut()?.apply(span, gain));
                    self.position = end;
                    if end == self.length {
                        self.enter(if self.boost.is_some() {
                            Stage::Drive
                        } else {
                            Stage::Fade
                        });
                    }
                }
                Stage::Peak => {
                    let end = (self.position + MASTER_FRAMES).min(self.length);
                    let span = samples.get_mut(self.position..end)?;
                    if self.boost.is_some() || self.gain == 1.0 {
                        for sample in span {
                            self.peak = self.peak.max(sample.abs());
                        }
                    } else {
                        for sample in span {
                            *sample *= self.gain;
                            self.peak = self.peak.max(sample.abs());
                        }
                    }
                    self.position = end;
                    if end == self.length {
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
                        self.enter(if self.filter.is_some() {
                            Stage::Fade
                        } else {
                            Stage::Scope
                        });
                    }
                }
                Stage::Fade => {
                    if self.fade != 0 {
                        let scale = (self.fade as f32).recip();
                        for (remaining, sample) in samples
                            .get_mut(..self.length)?
                            .iter_mut()
                            .rev()
                            .take(self.fade)
                            .enumerate()
                        {
                            *sample *= remaining as f32 * scale;
                        }
                    }
                    self.enter(Stage::Scope);
                }
                Stage::Scope => {
                    let end = (self.position + SCOPE_COLUMNS).min(Scope::COLUMNS);
                    let mix = samples.get_mut(..self.length)?;
                    let range = self.position..end;
                    let peak = if self.peak > 1.0 {
                        let trim = self.peak.recip();
                        scope.capture_columns(mix, job.rate, range, |sample| *sample *= trim)
                    } else {
                        scope.capture_columns(mix, job.rate, range, |_| {})
                    };
                    self.display_peak = self.display_peak.max(peak);
                    self.position = end;
                    if end == Scope::COLUMNS {
                        scope.finish_capture(self.display_peak);
                    }
                }
            }
            #[cfg(feature = "profiling")]
            self.record_timing(stage, timing, cpu_cycle_count().wrapping_sub(start));
            if matches!(stage, Stage::Scope) && self.position == Scope::COLUMNS {
                return Some(RenderStatus::Complete(self.length));
            }
            if cpu_cycle_count().wrapping_sub(slice_start) >= SLICE_CYCLES {
                return Some(RenderStatus::Pending);
            }
        }
    }

    fn render_layer(
        &mut self,
        job: &Job,
        cached: &mut [CachedLayer; MAX_LAYERS],
        buffers: &WaveBuffers,
        tables: Option<Lookup<'_>>,
        timing: &mut RenderTiming,
    ) -> Option<()> {
        while let Some(input) = self.inputs.get(self.position) {
            let index = self.position;
            self.position += 1;
            if let Some(input) = input {
                render_cached_layer(
                    &job.layers.get(index)?.patch,
                    job.rate,
                    index,
                    *input,
                    cached.get_mut(index)?,
                    buffers,
                    tables,
                    timing,
                )?;
                return Some(());
            }
        }
        self.enter(Stage::Mix);
        Some(())
    }

    fn mix(
        &mut self,
        buffers: &WaveBuffers,
        samples: &mut [f32],
        tile: &mut [f32],
        fused: bool,
    ) -> Option<()> {
        let start = self.position;
        if self.count >= MIN_TILED_LAYERS {
            let width = core::num::NonZeroUsize::new(tile.len())?;
            let end = (start + width.get()).min(self.dry_length);
            mix_tile(
                buffers,
                samples.get_mut(start..end)?,
                tile,
                start,
                &self.inputs,
            )?;
            self.position = end;
            return Some(());
        }
        let end = (start + MASTER_FRAMES).min(self.dry_length);
        let dest = samples.get_mut(start..end)?;
        let mut layers = self.inputs.iter().flatten();
        let first = *layers.next()?;
        if let Some(&second) = layers.next() {
            mix_pair(buffers, dest, start, first, second)?;
        } else {
            buffers.with(first.slot, |wave| {
                let wave = wave.get(start..end)?;
                if fused {
                    if first.level == 1.0 {
                        for (mixed, &sample) in dest.iter_mut().zip(wave) {
                            *mixed = (0.0 + sample) * self.gain;
                            self.peak = self.peak.max(mixed.abs());
                        }
                    } else {
                        for (mixed, &sample) in dest.iter_mut().zip(wave) {
                            *mixed = (0.0 + sample * first.level) * self.gain;
                            self.peak = self.peak.max(mixed.abs());
                        }
                    }
                } else {
                    mix_single(dest, wave, 0, first.level)?;
                }
                Some(())
            })??;
        }
        self.position = end;
        Some(())
    }

    #[cfg(feature = "profiling")]
    fn record_timing(&self, stage: Stage, timing: &mut RenderTiming, cycles: u32) {
        let field = match stage {
            Stage::Layers => return,
            Stage::Mix => {
                timing.mix = timing.mix.saturating_add(cycles);
                return;
            }
            Stage::Filter | Stage::Fade => &mut timing.lowpass,
            Stage::Drive if self.filter.is_some() => &mut timing.lowpass,
            Stage::Peak | Stage::Drive => &mut timing.gain_and_peak,
            Stage::Scope => &mut timing.scope,
        };
        *field = Some(field.unwrap_or(0).saturating_add(cycles));
    }
}
