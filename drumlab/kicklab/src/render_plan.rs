use super::drum_mix::TailTaper;
use super::{
    cpu_cycle_count, db_gain, layer_frames, layer_slot, mix_pair, mix_single, mix_tile,
    render_cached_layer, CachedLayer, Job, Lookup, Lowpass, MasterDrive, MixInput, Model,
    RenderTiming, WaveBuffers, MAX_LAYERS, MIN_TILED_LAYERS,
};
use crate::{Layer, Scope};

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
    peak: f32,
    display_peak: f32,
}

impl RenderPlan {
    pub fn new(job: &Job, max_frames: usize) -> Self {
        let solo = Layer::solo_active(&job.layers);
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
        let (filter, length) = if dry_length != 0 && cutoff > 0.0 {
            let tail = (job.rate as f32 * (3.0 / cutoff).max(0.005)) as usize + 1;
            let end = (dry_length + tail).min(max_frames);
            (Some(Lowpass::new(cutoff / job.rate as f32)), end)
        } else {
            (None, dry_length)
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
                            Stage::Scope
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
                        self.enter(Stage::Scope);
                    }
                }
                Stage::Scope => {
                    let end = (self.position + SCOPE_COLUMNS).min(Scope::COLUMNS);
                    let mix = samples.get_mut(..self.length)?;
                    let range = self.position..end;
                    let trim = if self.peak > 1.0 {
                        self.peak.recip()
                    } else {
                        1.0
                    };
                    let taper = if end > Scope::COLUMNS / 2 {
                        tail_taper(mix.len(), job.rate)
                    } else {
                        None
                    };
                    let peak = if let Some(taper) = taper {
                        scope.capture_columns(mix, job.rate, range, |sample, frame| {
                            *sample *= trim * taper.gain(frame);
                        })
                    } else if trim < 1.0 {
                        scope.capture_columns(mix, job.rate, range, |sample, _| *sample *= trim)
                    } else {
                        scope.capture_columns(mix, job.rate, range, |_, _| {})
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
            Stage::Filter => &mut timing.lowpass,
            Stage::Drive if self.filter.is_some() => &mut timing.lowpass,
            Stage::Peak | Stage::Drive => &mut timing.gain_and_peak,
            Stage::Scope => &mut timing.scope,
        };
        *field = Some(field.unwrap_or(0).saturating_add(cycles));
    }
}

fn tail_taper(length: usize, rate: u32) -> Option<TailTaper> {
    if length != rate as usize || length < 3 {
        return None;
    }
    Some(TailTaper::new(length / 2, length - 1))
}

#[cfg(test)]
mod tests {
    use super::tail_taper;

    #[test]
    fn full_length_taper_starts_at_half_a_second_and_ends_at_zero() {
        for rate in [32_000, 44_100, 48_000, 96_000, 192_000] {
            let frames = rate as usize;
            assert!(tail_taper(frames - 1, rate).is_none());
            let taper = tail_taper(frames, rate).unwrap();
            assert_eq!(taper.gain(frames / 2 - 1), 1.0);
            assert_eq!(taper.gain(frames / 2), 1.0);
            assert!((taper.gain(frames * 3 / 4) - 0.765625).abs() < 0.0001);
            assert!(taper.gain(frames - 2) < 0.000001);
            assert_eq!(taper.gain(frames - 1), 0.0);
        }
    }
}
