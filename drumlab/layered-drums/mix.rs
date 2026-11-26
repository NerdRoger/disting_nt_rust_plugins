use distingnt_api::api::math::exp2;

use crate::{WaveBuffers, MAX_LAYERS};

#[derive(Clone, Copy)]
pub(crate) struct MixInput {
    pub(crate) slot: usize,
    pub(crate) end: usize,
    pub(crate) level: f32,
}

distingnt_api::nt_dram_section! {
pub(crate) fn mix_pair(
    buffers: &WaveBuffers,
    mix: &mut [f32],
    start: usize,
    a: MixInput,
    b: MixInput,
) -> Option<()> {
    buffers.with(a.slot, |a_wave| {
        buffers.with(b.slot, |b_wave| {
            let a_end = a.end.saturating_sub(start).min(mix.len());
            let b_end = b.end.saturating_sub(start).min(mix.len());
            let a_wave = a_wave.get(start.min(a.end)..start.min(a.end) + a_end)?;
            let b_wave = b_wave.get(start.min(b.end)..start.min(b.end) + b_end)?;
            let shared = a_end.min(b_end);
            let (both, tail) = mix.split_at_mut_checked(shared)?;
            // Accumulate the first layer before the second to preserve rounding.
            for ((mixed, &a_sample), &b_sample) in both.iter_mut().zip(a_wave).zip(b_wave) {
                *mixed = (0.0 + a_sample * a.level) + b_sample * b.level;
            }
            let (longer, level) = if a_end > b_end { (a_wave, a.level) } else { (b_wave, b.level) };
            mix_single(tail, longer.get(shared..)?, 0, level)
        })?
    })?
}

pub(crate) fn mix_single(mix: &mut [f32], wave: &[f32], length: usize, level: f32) -> Option<()> {
    let overlap = length.min(mix.len());
    let (old, new) = mix.split_at_mut_checked(overlap)?;
    let (wave_old, wave_new) = wave.split_at_checked(overlap)?;
    for (mixed, &sample) in old.iter_mut().zip(wave_old) {
        *mixed += sample * level;
    }
    for (mixed, &sample) in new.iter_mut().zip(wave_new) {
        *mixed = 0.0 + sample * level;
    }
    Some(())
}

pub(crate) fn mix_tile(
    buffers: &WaveBuffers,
    dest: &mut [f32],
    tile: &mut [f32],
    start: usize,
    inputs: &[Option<MixInput>; MAX_LAYERS],
) -> Option<()> {
    // Each layer covers a prefix of the tile. Only overlap is accumulated.
    let mut written = 0;
    for input in inputs.iter().flatten() {
        let frames = input.end.saturating_sub(start).min(dest.len());
        if frames != 0 {
            let sum = tile.get_mut(..frames)?;
            buffers.with(input.slot, |wave| {
                mix_single(sum, wave.get(start..start + frames)?, written, input.level)
            })??;
            written = written.max(frames);
        }
    }
    dest.copy_from_slice(tile.get(..dest.len())?);
    Some(())

}
}

pub(crate) struct MasterDrive {
    boost: f32,
    ceiling: f32,
    gain: f32,
}

impl MasterDrive {
    pub(crate) fn new(boost: f32, undriven_peak: f32, gain: f32) -> Self {
        let ceiling = undriven_peak.max(1.0);
        // Boost and clipping are monotonic in magnitude, so the undriven
        // peak also determines the driven peak and the final safety trim.
        let driven_peak = (undriven_peak * boost).min(ceiling) * gain;
        let gain = if driven_peak > 1.0 {
            gain * driven_peak.recip()
        } else {
            gain
        };
        Self {
            boost,
            ceiling,
            gain,
        }
    }

    pub(crate) fn apply(&self, samples: &mut [f32]) {
        for sample in samples {
            *sample = (*sample * self.boost).max(-self.ceiling).min(self.ceiling) * self.gain;
        }
    }
}

pub(crate) fn db_gain(db: f32) -> f32 {
    exp2(db * (core::f32::consts::LOG2_10 / 20.0))
}

#[derive(Clone, Copy)]
pub(crate) struct TailTaper {
    start: usize,
    last: usize,
    step: f32,
}

impl TailTaper {
    pub(crate) fn new(start: usize, last: usize) -> Self {
        Self {
            start,
            last,
            step: ((last - start) as f32).recip(),
        }
    }

    pub(crate) fn gain(&self, frame: usize) -> f32 {
        if frame >= self.last {
            return 0.0;
        }
        let progress = frame.saturating_sub(self.start) as f32 * self.step;
        // Squaring the cubic complement gives zero slope at the last sample.
        let remaining = 1.0 - progress * progress * progress;
        remaining * remaining
    }
}
