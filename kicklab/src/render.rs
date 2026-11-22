//! Mono KickLab renderers. These run only on the background worker.
#[path = "extra_models.rs"]
mod extra_models;
#[path = "spectral.rs"]
mod spectral;

use core::f32::consts::{LOG2_E, PI, TAU};
use distingnt_api::api::math::exp2;

use crate::lookup::{self, Lookup};
use crate::profile::{GrainTiming, RenderTiming};
use crate::{
    layer_slot, Job, LayerPatch, Model, WaveBuffers, GRAIN_SIZE_MAX_MS, MAX_LAYERS,
    PITCH_CLICK_MAX_DECAY_TENTHS_MS,
};
#[cfg(feature = "profiling")]
use distingnt_api::api::cpu_cycle_count;

const RELEASE_FADE_MS: usize = 5;

pub fn max_frames(rate: u32) -> usize {
    rate as usize
}

fn milliseconds(rate: u32, ms: usize) -> usize {
    (rate as usize * ms).div_ceil(1000)
}

fn max_click_frames(rate: u32) -> usize {
    // 3.5 times the maximum Click Decay, which is stored in tenths of a ms.
    (7 * PITCH_CLICK_MAX_DECAY_TENTHS_MS as usize * rate as usize).div_ceil(20_000)
}

pub fn layer_cache_frames(rate: u32) -> usize {
    // Each model can use this space for its own caches. Reserve enough for
    // the largest layout so layers can switch models without reallocating.
    (2 * max_click_frames(rate).max(max_beater_frames(rate)))
        .max(milliseconds(rate, GRAIN_SIZE_MAX_MS as usize))
}

#[derive(Clone, Copy)]
pub struct CachedLayer {
    patch: LayerPatch,
    transient_shape: TransientShape,
}

#[derive(Clone, Copy, PartialEq)]
enum TransientShape {
    Empty,
    PitchSweep { tone: f32, decay: f32 },
    ModalDrum { hardness: f32 },
    GrainCloud { size_ms: f32, raw_frames: usize },
}

impl CachedLayer {
    pub const EMPTY: Self = Self {
        patch: LayerPatch::OFF,
        transient_shape: TransientShape::Empty,
    };
}

struct MixInput {
    slot: usize,
    end: usize,
    level: f32,
}

distingnt_api::nt_dram_section! {
pub fn render(
    job: &Job,
    cached_layers: &mut [CachedLayer; MAX_LAYERS],
    buffers: &WaveBuffers,
    samples: &mut [f32],
    tables: Option<Lookup<'_>>,
    _timing: &mut RenderTiming,
    scope: &mut crate::Scope,
) -> Option<usize> {
    let rate = job.rate;
    let cutoff = (job.cutoff as f32).min(rate as f32 * 0.45);
    let gain = db_gain(job.gain);
    let drive = (job.drive > 0.0).then(|| db_gain(job.drive));
    let solo_active = job.layers.iter().any(|layer| layer.patch.model != Model::Off && layer.solo);
    let fuse_mix = cutoff == 0.0 && drive.is_none()
        && job.layers.iter()
            .filter(|layer| layer.audible(solo_active))
            .take(2)
            .count() == 1;
    let mut peak = 1.0f32;
    let mut length = 0;
    let mut pending: Option<MixInput> = None;
    for (index, (layer, cached)) in job.layers.iter().zip(cached_layers).enumerate() {
        if !layer.audible(solo_active) {
            continue;
        }
        let patch = &layer.patch;
        let end = milliseconds(rate, patch.controls[1] as usize * 3 + 10)
            .div_ceil(2)
            .min(samples.len());
        // Layer caches start after the output pool. The cache holds post-drive,
        // pre-level samples; the sample rate is fixed for this instance.
        let slot = layer_slot(usize::from(job.output_buffers), index);
        buffers.with(slot, |cache| {
            let waveform = cache.get_mut(..end)?;
            if cached.patch != *patch {
                if cached.patch.model != patch.model {
                    cached.transient_shape = TransientShape::Empty;
                }
                #[cfg(feature = "profiling")]
                let start = cpu_cycle_count();
                match patch.model {
                    Model::Off => {}
                    Model::PitchSweep => buffers.with(slot + 1, |layer_cache| {
                        pitch_sweep(patch, cached, rate, waveform, layer_cache, tables)
                    })??,
                    Model::ModalDrum => buffers.with(slot + 1, |layer_cache| {
                        modal_drum(patch, cached, rate, waveform, layer_cache, tables)
                    })??,
                    Model::BitLogic => bit_logic(patch, rate, waveform, tables),
                    Model::GrainCloud => {
                        let mut timing = GrainTiming::default();
                        buffers.with(slot + 1, |layer_cache| {
                            grain_cloud(patch, cached, rate, waveform, layer_cache, tables, &mut timing)
                        })??;
                        #[cfg(feature = "profiling")]
                        if let Some(result) = _timing.grains.get_mut(index) {
                            *result = timing;
                        }
                    }
                    Model::HardSync => hard_sync(patch, rate, waveform, tables),
                    Model::Resonator => extra_models::resonator(patch, rate, waveform, tables),
                    Model::TwoOperatorFm => extra_models::two_operator_fm(patch, rate, waveform, tables),
                    Model::NoiseSweep => extra_models::noise_sweep(patch, rate, waveform, tables),
                    Model::RingMod => extra_models::ring_mod(patch, rate, waveform, tables),
                    Model::Wavefold => extra_models::wavefold(patch, rate, waveform, tables),
                    Model::DelayDrum => buffers.with(slot + 1, |layer_cache| {
                        extra_models::delay_drum(patch, rate, waveform, layer_cache, tables)
                    })??,
                    Model::SpectralPulse => spectral::spectral_pulse(patch, rate, waveform, tables),
                }
                #[cfg(feature = "profiling")]
                {
                    let elapsed = cpu_cycle_count().wrapping_sub(start);
                    if let Some(timing) = _timing.synthesis.get_mut(index) {
                        *timing = Some(elapsed);
                    }
                }
                store_patch(&mut cached.patch, patch);
            }
            if fuse_mix {
                #[cfg(feature = "profiling")]
                let start = cpu_cycle_count();
                let mix = samples.get_mut(..end)?;
                if layer.level == 1.0 {
                    for (mixed, &sample) in mix.iter_mut().zip(waveform.iter()) {
                        let scaled = (0.0 + sample) * gain;
                        *mixed = scaled;
                        peak = peak.max(scaled.abs());
                    }
                } else {
                    for (mixed, &sample) in mix.iter_mut().zip(waveform.iter()) {
                        let scaled = (0.0 + sample * layer.level) * gain;
                        *mixed = scaled;
                        peak = peak.max(scaled.abs());
                    }
                }
                #[cfg(feature = "profiling")]
                {
                    _timing.mix += cpu_cycle_count().wrapping_sub(start);
                }
            }
            Some(())
        })??;
        if fuse_mix {
            length = end;
            continue;
        }
        let input = MixInput { slot, end, level: layer.level };
        if let Some(first) = pending.take() {
            #[cfg(feature = "profiling")]
            let start = cpu_cycle_count();
            let end = first.end.max(input.end);
            mix_pair(buffers, samples.get_mut(..end)?, length, first, input)?;
            length = length.max(end);
            #[cfg(feature = "profiling")]
            {
                _timing.mix += cpu_cycle_count().wrapping_sub(start);
            }
        } else {
            pending = Some(input);
        }
    }
    if let Some(input) = pending {
        #[cfg(feature = "profiling")]
        let start = cpu_cycle_count();
        let mix = samples.get_mut(..input.end)?;
        buffers.with(input.slot, |wave| {
            mix_single(mix, wave.get(..input.end)?, length, input.level)
        })??;
        length = length.max(input.end);
        #[cfg(feature = "profiling")]
        {
            _timing.mix += cpu_cycle_count().wrapping_sub(start);
        }
    }
    if length == 0 {
        scope.capture(&mut [], rate, |_| {});
        return Some(0);
    }
    if cutoff > 0.0 {
        let tail = (rate as f32 * (3.0 / cutoff).max(0.005)) as usize + 1;
        let requested_end = length + tail;
        let end = requested_end.min(samples.len());
        let mix = samples.get_mut(..end)?;
        mix.iter_mut().skip(length).for_each(|sample| *sample = 0.0);
        length = end;
        #[cfg(feature = "profiling")]
        let start = cpu_cycle_count();
        let cycles = cutoff / rate as f32;
        let fade = if end < requested_end {
            milliseconds(rate, RELEASE_FADE_MS).min(end)
        } else {
            0
        };
        peak = match drive {
            Some(boost) => {
                let undriven_peak = lowpass(mix, cycles, 1.0);
                MasterDrive::new(boost, undriven_peak, gain).apply(mix);
                1.0
            }
            None => lowpass(mix, cycles, gain),
        };
        if fade != 0 {
            // Fade the filter tail if it would run past the cap.
            let scale = (fade as f32).recip();
            for (remaining, sample) in mix.iter_mut().rev().take(fade).enumerate() {
                *sample *= remaining as f32 * scale;
            }
        }
        #[cfg(feature = "profiling")]
        {
            _timing.lowpass = Some(cpu_cycle_count().wrapping_sub(start));
        }
    }
    let mix = samples.get_mut(..length)?;
    #[cfg(feature = "profiling")]
    let start = cpu_cycle_count();
    // The filter and the single-layer mix already applied gain and tracked
    // the peak while writing. An unfiltered multi-layer mix needs this pass.
    if cutoff == 0.0 && !fuse_mix {
        if let Some(boost) = drive {
            let undriven_peak = mix.iter().fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            MasterDrive::new(boost, undriven_peak, gain).apply(mix);
        } else if gain == 1.0 {
            for sample in &*mix {
                peak = peak.max(sample.abs());
            }
        } else {
            for sample in &mut *mix {
                *sample *= gain;
                peak = peak.max(sample.abs());
            }
        }
    }
    #[cfg(feature = "profiling")]
    {
        _timing.gain_and_peak = Some(cpu_cycle_count().wrapping_sub(start));
    }
    #[cfg(feature = "profiling")]
    let start = cpu_cycle_count();
    if peak > 1.0 {
        let trim = peak.recip();
        scope.capture(mix, rate, |sample| *sample *= trim);
    } else {
        scope.capture(mix, rate, |_| {});
    }
    #[cfg(feature = "profiling")]
    {
        _timing.scope = Some(cpu_cycle_count().wrapping_sub(start));
    }
    Some(length)
}

fn mix_pair(
    buffers: &WaveBuffers,
    mix: &mut [f32],
    length: usize,
    a: MixInput,
    b: MixInput,
) -> Option<()> {
    buffers.with(a.slot, |a_wave| {
        buffers.with(b.slot, |b_wave| {
            let a_wave = a_wave.get(..a.end)?;
            let b_wave = b_wave.get(..b.end)?;
            let shared = a.end.min(b.end);
            let (both, tail) = mix.split_at_mut_checked(shared)?;
            let overlap = length.min(shared);
            let (old, new) = both.split_at_mut_checked(overlap)?;
            let (a_old, a_new) = a_wave.get(..shared)?.split_at_checked(overlap)?;
            let (b_old, b_new) = b_wave.get(..shared)?.split_at_checked(overlap)?;
            // Accumulate the first layer before the second to preserve rounding.
            for ((mixed, &a_sample), &b_sample) in old.iter_mut().zip(a_old).zip(b_old) {
                *mixed = (*mixed + a_sample * a.level) + b_sample * b.level;
            }
            for ((mixed, &a_sample), &b_sample) in new.iter_mut().zip(a_new).zip(b_new) {
                *mixed = (0.0 + a_sample * a.level) + b_sample * b.level;
            }
            let (longer, level) = if a.end > b.end { (a_wave, a.level) } else { (b_wave, b.level) };
            mix_single(tail, longer.get(shared..)?, length.saturating_sub(shared), level)
        })?
    })?
}

fn mix_single(mix: &mut [f32], wave: &[f32], length: usize, level: f32) -> Option<()> {
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
}

// The call boundary preserves the disjoint borrows for this small copy.
#[inline(never)]
fn store_patch(destination: &mut LayerPatch, source: &LayerPatch) {
    *destination = *source;
}

struct MasterDrive {
    boost: f32,
    ceiling: f32,
    gain: f32,
}

impl MasterDrive {
    fn new(boost: f32, undriven_peak: f32, gain: f32) -> Self {
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

    fn apply(&self, samples: &mut [f32]) {
        for sample in samples {
            *sample = (*sample * self.boost).max(-self.ceiling).min(self.ceiling) * self.gain;
        }
    }
}

fn lowpass(samples: &mut [f32], cycles: f32, gain: f32) -> f32 {
    let k = sine(cycles * 0.5) / sine(0.25 + cycles * 0.5);
    let squared = k * k;
    let root_two_k = core::f32::consts::SQRT_2 * k;
    let norm = (1.0 + root_two_k + squared).recip();
    let b0 = squared * norm;
    let b1 = 2.0 * b0;
    let a1 = 2.0 * (squared - 1.0) * norm;
    let a2 = (1.0 - root_two_k + squared) * norm;
    let mut z1 = 0.0;
    let mut z2 = 0.0;
    let mut peak = 0.0_f32;
    for sample in samples {
        let output = *sample * b0 + z1;
        z1 = *sample * b1 - output * a1 + z2;
        z2 = *sample * b0 - output * a2;
        let scaled = output * gain;
        *sample = scaled;
        peak = peak.max(scaled.abs());
    }
    peak
}

fn db_gain(db: f32) -> f32 {
    exp2(db * (core::f32::consts::LOG2_10 / 20.0))
}

fn decay_rate(milliseconds: f32, rate: u32) -> f32 {
    -2.0 * core::f32::consts::LOG2_10 * 1000.0 / (milliseconds * rate as f32)
}

fn filter_coefficient(hz: f32, rate: u32) -> f32 {
    1.0 - exp2(-TAU * LOG2_E * hz / rate as f32)
}

fn wrap(turns: f32) -> f32 {
    turns - turns as u32 as f32
}

#[inline(never)]
fn sine(turns: f32) -> f32 {
    // Reduce to +/-pi/2 before evaluating the odd polynomial.
    let angle = (4.0 * (wrap(turns + 0.25) - 0.5).abs() - 1.0) * (-PI * 0.5);
    let squared = angle * angle;
    angle
        * (1.0
            + squared
                * (-1.0 / 6.0
                    + squared * (1.0 / 120.0 + squared * (-1.0 / 5040.0 + squared / 362880.0))))
}

fn positive_pow(value: f32, power: f32) -> f32 {
    if value == 0.0 {
        return 0.0;
    }
    // Decompose into a power of two and a mantissa in [1, 2).
    let bits = value.to_bits();
    let exponent = (bits >> 23) as i32 - 127;
    let mantissa = f32::from_bits((bits & 0x7f_ffff) | 0x3f80_0000);
    let x = (mantissa - 1.0) / (mantissa + 1.0);
    let squared = x * x;
    let log = 2.0
        * x
        * (1.0
            + squared
                * (1.0 / 3.0 + squared * (1.0 / 5.0 + squared * (1.0 / 7.0 + squared / 9.0))));
    exp2((exponent as f32 + LOG2_E * log) * power)
}

struct NoiseBand {
    seed: u32,
    low: f32,
    high: f32,
    low_coefficient: f32,
    high_coefficient: f32,
}

const NOISE_SEED: u32 = 0x56d62e11;

fn next_noise_bits(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

impl NoiseBand {
    fn new(low: f32, high: f32, rate: u32) -> Self {
        Self {
            seed: NOISE_SEED,
            low: 0.0,
            high: 0.0,
            low_coefficient: filter_coefficient(low, rate),
            high_coefficient: filter_coefficient(high, rate),
        }
    }

    fn next(&mut self) -> f32 {
        let sample = next_noise_bits(&mut self.seed) as f32 / 2147483648.0 - 1.0;
        self.low += self.low_coefficient * (sample - self.low);
        self.high += self.high_coefficient * (sample - self.high);
        2.0 * (self.high - self.low)
    }
}

struct LayerShape<'a> {
    boost: f32,
    blend: f32,
    attack: f32,
    fade: f32,
    last: usize,
    tables: Option<Lookup<'a>>,
}

impl<'a> LayerShape<'a> {
    fn new(drive: f32, rate: u32, length: usize, tables: Option<Lookup<'a>>) -> Self {
        Self {
            boost: db_gain(drive),
            blend: (drive / 5.0).min(1.0),
            attack: (rate as f32 * 0.0007).recip(),
            fade: (rate as f32 * (RELEASE_FADE_MS as f32 / 1000.0)).recip(),
            last: length.saturating_sub(1),
            tables,
        }
    }

    fn sample(&self, frame: usize, input: f32) -> f32 {
        let raw = input * (frame as f32 * self.attack).min(1.0);
        self.drive(raw) * ((self.last - frame) as f32 * self.fade).min(1.0)
    }

    #[inline]
    fn drive(&self, mut raw: f32) -> f32 {
        if self.blend > 0.0 {
            let saturated = lookup::drive(self.tables, raw * self.boost);
            raw += (saturated - raw) * self.blend;
        }
        raw
    }
}

fn render_layer(
    drive: f32,
    rate: u32,
    waveform: &mut [f32],
    tables: Option<Lookup<'_>>,
    mut synth: impl FnMut(usize) -> f32,
) {
    let shape = LayerShape::new(drive, rate, waveform.len(), tables);
    // Allow for the full 0.7 ms attack and round up the release span.
    let onset = milliseconds(rate, 1);
    let steady_end = waveform
        .len()
        .saturating_sub(milliseconds(rate, RELEASE_FADE_MS) + 1);
    for (frame, sample) in waveform.iter_mut().enumerate() {
        let raw = synth(frame);
        *sample = if frame >= onset && frame < steady_end {
            shape.drive(raw)
        } else {
            shape.sample(frame, raw)
        };
    }
}

struct DecayEnvelope {
    log2_step: f32,
    ratio: f32,
    value: f32,
}

impl DecayEnvelope {
    fn new(log2_step: f32) -> Self {
        Self {
            log2_step,
            ratio: exp2(log2_step),
            value: 1.0,
        }
    }

    #[inline(never)]
    fn next<const ANCHOR: usize>(&mut self, index: usize) -> f32 {
        // Re-anchor periodically so rounding in the ratio cannot drift over
        // a long render, especially at the highest supported sample rates.
        if index & (ANCHOR - 1) == 0 && index != 0 {
            self.anchor(index);
        }
        self.advance()
    }

    fn anchor(&mut self, index: usize) {
        self.value = exp2(index as f32 * self.log2_step);
    }

    #[inline]
    fn advance(&mut self) -> f32 {
        let value = self.value;
        self.value *= self.ratio;
        value
    }
}

enum SweepEnvelope {
    Linear {
        decay: DecayEnvelope,
        stride: usize,
        knot: usize,
    },
    Curved {
        frames: f32,
        log2_step: f32,
        curvature: f32,
    },
}

impl SweepEnvelope {
    fn new(sweep_time: f32, curve: f32, rate: u32) -> Self {
        let frames = sweep_time * rate as f32 / 1000.0;
        let log2_step = decay_rate(sweep_time, rate);
        if curve == 0.0 {
            let stride = (frames as usize / 512).clamp(1, 8);
            let mut decay = DecayEnvelope::new(log2_step * stride as f32);
            decay.next::<16>(0);
            Self::Linear {
                decay,
                stride,
                knot: 0,
            }
        } else {
            Self::Curved {
                frames,
                log2_step,
                curvature: exp2(curve / 50.0),
            }
        }
    }

    fn next_span(&mut self, frame: usize) -> (usize, f32, f32) {
        match self {
            Self::Linear {
                decay,
                stride,
                knot,
            } => {
                let midpoint = decay.next::<16>(*knot + 1);
                let end = decay.next::<16>(*knot + 2);
                *knot += 2;
                (*stride * 2, midpoint, end)
            }
            Self::Curved {
                frames,
                log2_step,
                curvature,
            } => {
                // Start at single-sample spacing near the onset, where steep
                // curves can change rapidly. Increase spacing with elapsed time.
                let half = (frame / 512).clamp(1, 8);
                let level = |index: usize| {
                    let position = *frames * positive_pow(index as f32 / *frames, *curvature);
                    exp2(position * *log2_step)
                };
                (half * 2, level(frame + half), level(frame + half * 2))
            }
        }
    }
}

struct PitchSweep {
    envelope: Option<SweepEnvelope>,
    base_increment: f32,
    tune: f32,
    rate: f32,
    depth_octaves: f32,
    increment: f32,
    target: f32,
    delta: f32,
    delta_step: f32,
    next_knot: usize,
}

impl PitchSweep {
    fn new(
        tune: f32,
        depth: f32,
        sweep_time: f32,
        curve: f32,
        rate: u32,
        tables: Option<Lookup<'_>>,
    ) -> Self {
        let base_increment = tune / rate as f32;
        let mut sweep = Self {
            envelope: None,
            base_increment,
            tune,
            rate: rate as f32,
            depth_octaves: depth / 12.0,
            increment: base_increment,
            target: base_increment,
            delta: 0.0,
            delta_step: 0.0,
            next_knot: usize::MAX,
        };
        if depth > 0.0 {
            sweep.envelope = Some(SweepEnvelope::new(sweep_time, curve, rate));
            sweep.increment =
                tune * lookup::pitch_multiplier(tables, sweep.depth_octaves) / rate as f32;
            sweep.target = sweep.increment;
            sweep.next_knot = 0;
        }
        sweep
    }

    #[inline]
    fn next_increment(&mut self, frame: usize, tables: Option<Lookup<'_>>) -> f32 {
        if frame == self.next_knot {
            self.increment = self.target;
            if self.target == self.base_increment {
                self.next_knot = usize::MAX;
            } else if let Some(envelope) = &mut self.envelope {
                let (stride, mid_level, end_level) = envelope.next_span(frame);
                let to_increment = |level: f32| {
                    let offset = self.depth_octaves * level;
                    if offset <= 0.01 / 1200.0 {
                        self.base_increment
                    } else {
                        self.tune * lookup::pitch_multiplier(tables, offset) / self.rate
                    }
                };
                let midpoint = to_increment(mid_level);
                self.target = to_increment(end_level);
                // Quadratic through the start, midpoint and end. Integrating
                // a linear estimate can accumulate an audible phase error
                // in long, deep sweeps. Advance with two adds per sample.
                let first = midpoint - self.increment;
                let second = self.target - midpoint;
                let inverse_stride = (stride as f32).recip();
                let quadratic = 2.0 * (second - first) * inverse_stride * inverse_stride;
                self.delta = (3.0 * first - second) * inverse_stride + quadratic;
                self.delta_step = 2.0 * quadratic;
                self.next_knot = frame + stride;
            }
        }
        if self.next_knot == usize::MAX {
            return self.base_increment;
        }
        let increment = self.increment;
        self.increment += self.delta;
        self.delta += self.delta_step;
        increment
    }
}

fn click_frames(layer: &LayerPatch, rate: u32) -> usize {
    if layer.controls[6] > 0.0 {
        // Decay time is specified at -40 dB. End the click at -140 dB.
        (3.5 * layer.controls[8] * rate as f32 / 1000.0) as usize
    } else {
        0
    }
}

fn pitch_sweep(
    layer: &LayerPatch,
    cached: &mut CachedLayer,
    rate: u32,
    waveform: &mut [f32],
    layer_cache: &mut [f32],
    tables: Option<Lookup<'_>>,
) -> Option<()> {
    let [tune, decay, drive, sweep, sweep_time, curve, click_level, click_tone, click_decay, _] =
        layer.controls;
    let (body_prefix, click_storage) = layer_cache.split_at_mut_checked(max_click_frames(rate))?;
    let click = click_storage.get_mut(..click_frames(layer, rate))?;
    let click_shape = TransientShape::PitchSweep {
        tone: click_tone,
        decay: click_decay,
    };
    if !click.is_empty() && cached.transient_shape != click_shape {
        let mut noise = NoiseBand::new(900.0, click_tone, rate);
        let mut envelope = DecayEnvelope::new(decay_rate(click_decay, rate));
        for (frame, sample) in click.iter_mut().enumerate() {
            *sample = noise.next() * envelope.next::<256>(frame);
        }
        cached.transient_shape = click_shape;
    }

    let click_gain = click_level / 100.0;
    let input = |frame, body| body + click.get(frame).map_or(0.0, |sample| *sample * click_gain);
    let shape = LayerShape::new(drive, rate, waveform.len(), tables);
    if cached.patch.model == Model::PitchSweep && cached.patch.controls[..6] == layer.controls[..6]
    {
        // Include the previous span so reducing the decay or muting the
        // click removes its contribution all the way to its old endpoint.
        let end = click_frames(&cached.patch, rate)
            .max(click.len())
            .min(waveform.len());
        let changed = waveform.get_mut(..end)?;
        for (frame, (sample, &body)) in changed.iter_mut().zip(body_prefix.iter()).enumerate() {
            *sample = shape.sample(frame, input(frame, body));
        }
        return Some(());
    }

    let mut body_envelope = DecayEnvelope::new(decay_rate(decay, rate));
    let mut sweep = PitchSweep::new(tune, sweep, sweep_time, curve, rate, tables);
    let mut phase = 0.0;
    let prefix_len = body_prefix.len().min(waveform.len());
    let (prefix, tail) = waveform.split_at_mut_checked(prefix_len)?;
    let mut next_body = |frame| {
        let body = sine(phase) * body_envelope.next::<256>(frame);
        phase = wrap(phase + sweep.next_increment(frame, tables));
        body
    };
    for (frame, (sample, cached_body)) in prefix.iter_mut().zip(body_prefix).enumerate() {
        let body = next_body(frame);
        *cached_body = body;
        *sample = shape.sample(frame, input(frame, body));
    }
    // The cached prefix spans the attack. In the tail, the middle section
    // also precedes the fade, so its only shaping operation is Drive.
    let unfaded = tail
        .len()
        .saturating_sub(milliseconds(rate, RELEASE_FADE_MS) + 1);
    let (middle, fade) = tail.split_at_mut(unfaded);
    let mut frame = prefix_len;
    let mut middle_samples = middle.iter_mut();
    while sweep.next_knot != usize::MAX {
        let Some(sample) = middle_samples.next() else {
            break;
        };
        *sample = shape.drive(sine(phase) * body_envelope.next::<256>(frame));
        phase = wrap(phase + sweep.next_increment(frame, tables));
        frame += 1;
    }
    let mut steady = middle_samples.into_slice();
    if !steady.is_empty() {
        let step = sweep.base_increment;
        let step_sine = sine(step);
        let half_sine = sine(step * 0.5);
        let step_cosine = 1.0 - 2.0 * half_sine * half_sine;
        const ANCHOR: usize = 256;
        while !steady.is_empty() {
            let offset = frame & (ANCHOR - 1);
            if offset == 0 {
                body_envelope.anchor(frame);
            }
            // Start the oscillator at the exact accumulated phase. The
            // first span may begin partway through an envelope interval.
            let count = steady.len().min(ANCHOR - offset);
            let (span, rest) = steady.split_at_mut_checked(count)?;
            let (mut y, mut x) = (sine(phase), sine(phase + 0.25));
            for sample in span {
                *sample = shape.drive(y * body_envelope.advance());
                (y, x) = (
                    y * step_cosine + x * step_sine,
                    x * step_cosine - y * step_sine,
                );
                phase = wrap(phase + step);
            }
            frame += count;
            steady = rest;
        }
    }
    for sample in fade {
        let body = sine(phase) * body_envelope.next::<256>(frame);
        phase = wrap(phase + sweep.next_increment(frame, tables));
        *sample = shape.sample(frame, body);
        frame += 1;
    }
    Some(())
}

fn modal_modes(layer: &LayerPatch, rate: u32) -> [(f32, f32, f32); 5] {
    let [tune, decay, _, resonance, damping, strike, ..] = layer.controls;
    let modes = [
        (1.0, 0.86, 1.0, -0.55),
        (1.59, 0.57, 0.66, 0.7),
        (2.14, 0.37, 0.45, -0.4),
        (2.92, 0.21, 0.25, 0.9),
        (3.60, 0.12, 0.17, 0.8),
    ];
    let position = (strike - 50.0) / 50.0;
    core::array::from_fn(|index| {
        let (ratio, mut level, length, edge) = modes[index];
        let mut duration = decay;
        if index != 0 {
            level *= resonance / 100.0;
            duration *= length / (0.5 + damping / 55.0);
        }
        (
            ratio * tune / rate as f32,
            decay_rate(duration, rate),
            level * (1.0 + position * edge),
        )
    })
}

struct DampedMode {
    phase_step: f32,
    log2_step: f32,
    level: f32,
    sin_step: f32,
    cos_step: f32,
    sin: f32,
    cos: f32,
}

impl DampedMode {
    fn new((phase_step, log2_step, level): (f32, f32, f32)) -> Self {
        let ratio = exp2(log2_step);
        let half_sine = sine(phase_step * 0.5);
        Self {
            phase_step,
            log2_step,
            level,
            sin_step: sine(phase_step) * ratio,
            cos_step: (1.0 - 2.0 * half_sine * half_sine) * ratio,
            sin: 0.0,
            cos: level,
        }
    }

    fn anchor(&mut self, frame: usize) {
        let phase = self.phase_step * frame as f32;
        let amplitude = self.level * exp2(self.log2_step * frame as f32);
        self.sin = sine(phase) * amplitude;
        self.cos = sine(phase + 0.25) * amplitude;
    }

    fn next(&mut self) -> f32 {
        let sample = self.sin;
        self.sin = sample * self.cos_step + self.cos * self.sin_step;
        self.cos = self.cos * self.cos_step - sample * self.sin_step;
        sample
    }
}

const MODAL_ANCHOR: usize = 256;

fn max_beater_frames(rate: u32) -> usize {
    // Softest beater: 13 ms at -40 dB, or 45.5 ms at -140 dB.
    (91 * rate as usize).div_ceil(2000)
}

fn beater_frames(layer: &LayerPatch, rate: u32) -> usize {
    if layer.controls[6] > 0.0 {
        (3.5 * (13.0 - layer.controls[7] * 0.1) * rate as f32 / 1000.0) as usize
    } else {
        0
    }
}

fn modal_drum(
    layer: &LayerPatch,
    cached: &mut CachedLayer,
    rate: u32,
    waveform: &mut [f32],
    layer_cache: &mut [f32],
    tables: Option<Lookup<'_>>,
) -> Option<()> {
    let [_, _, drive, upper_modes, _, _, beater, hardness, ..] = layer.controls;
    let (body_prefix, beater_storage) =
        layer_cache.split_at_mut_checked(max_beater_frames(rate))?;
    let strike = beater_storage.get_mut(..beater_frames(layer, rate))?;
    let beater_shape = TransientShape::ModalDrum { hardness };
    if !strike.is_empty() && cached.transient_shape != beater_shape {
        let mut noise = NoiseBand::new(650.0, 1200.0 + hardness * 72.0, rate);
        let mut envelope = DecayEnvelope::new(decay_rate(13.0 - hardness * 0.1, rate));
        for (frame, sample) in strike.iter_mut().enumerate() {
            *sample = noise.next() * envelope.next::<256>(frame);
        }
        cached.transient_shape = beater_shape;
    }
    let shape = LayerShape::new(drive, rate, waveform.len(), tables);
    if cached.patch.model == Model::ModalDrum && cached.patch.controls[..6] == layer.controls[..6] {
        // Cover both the old and the new transient when the beater gets
        // shorter or turns off. The rest of the waveform is still valid.
        let end = strike
            .len()
            .max(beater_frames(&cached.patch, rate))
            .min(waveform.len());
        for (frame, (sample, &body)) in waveform
            .get_mut(..end)?
            .iter_mut()
            .zip(body_prefix.iter())
            .enumerate()
        {
            let transient = strike
                .get(frame)
                .map_or(0.0, |&noise| noise * beater / 70.0);
            *sample = shape.sample(frame, body + transient);
        }
        return Some(());
    }

    let modes = modal_modes(layer, rate);
    let fade_start = waveform
        .len()
        .saturating_sub(milliseconds(rate, RELEASE_FADE_MS) + 1);
    if upper_modes == 0.0 {
        let [fundamental, ..] = modes;
        render_modal_body(
            &mut [DampedMode::new(fundamental)],
            waveform,
            body_prefix,
            strike,
            beater,
            &shape,
            fade_start,
        )
    } else {
        render_modal_body(
            &mut modes.map(DampedMode::new),
            waveform,
            body_prefix,
            strike,
            beater,
            &shape,
            fade_start,
        )
    }
}

fn render_modal_body<const N: usize>(
    modes: &mut [DampedMode; N],
    waveform: &mut [f32],
    body_prefix: &mut [f32],
    strike: &[f32],
    beater: f32,
    shape: &LayerShape<'_>,
    fade_start: usize,
) -> Option<()> {
    // Always cache the full possible onset so a later beater edit can
    // lengthen the transient without re-rendering the resonant modes.
    let mut body_prefix = body_prefix.iter_mut();
    let mut strike = strike.iter();
    let mut frame = 0;
    for span in waveform.chunks_mut(MODAL_ANCHOR) {
        if frame != 0 {
            for mode in modes.iter_mut() {
                mode.anchor(frame);
            }
        }
        let (onset, tail) = span.split_at_mut_checked(body_prefix.len().min(span.len()))?;
        for (sample, cached_body) in onset.iter_mut().zip(&mut body_prefix) {
            let body = modes.iter_mut().map(DampedMode::next).sum::<f32>();
            *cached_body = body;
            let transient = strike.next().map_or(0.0, |&noise| noise * beater / 70.0);
            *sample = shape.sample(frame, body + transient);
            frame += 1;
        }
        // The cached onset spans the attack, so the middle needs only Drive.
        let steady_frames = tail.len().min(fade_start.saturating_sub(frame));
        let (steady, fade) = tail.split_at_mut_checked(steady_frames)?;
        for sample in steady {
            let body = modes.iter_mut().map(DampedMode::next).sum::<f32>();
            *sample = shape.drive(body);
        }
        frame += steady_frames;
        for sample in fade {
            let body = modes.iter_mut().map(DampedMode::next).sum::<f32>();
            *sample = shape.sample(frame, body);
            frame += 1;
        }
    }
    Some(())
}

fn grain_schedule(layer: &LayerPatch, rate: u32) -> impl Iterator<Item = (usize, f32, f32)> {
    let [tune, _, sweep, sweep_time, _, density, density_depth, spread, jitter, _] = layer.controls;
    let sweep_decay = decay_rate(sweep_time, rate);
    let rate_hz = f64::from(rate);
    let mut next_time = 0.0f64;
    let mut seed = NOISE_SEED;
    let mut random = move || f64::from(next_noise_bits(&mut seed)) / 2147483648.0 - 1.0;
    core::iter::from_fn(move || {
        let frame = (next_time * rate_hz) as usize;
        let frame = frame + usize::from((frame as f64 / rate_hz) < next_time);
        let sweep_level = exp2(frame as f32 * sweep_decay);
        let detune = random() as f32 * spread / 12.0;
        let frequency = tune * exp2(sweep / 12.0 * sweep_level) * exp2(detune);
        // The source phase is signed. Offset by one turn before calling sine.
        let phase = 1.0 + random() as f32 * spread / 48.0;
        let grain_rate = (density * exp2(density_depth * sweep_level))
            .max(6.0)
            .min(350.0);
        let spacing = (1.0 + 0.4 * random() * f64::from(jitter) / 100.0) / f64::from(grain_rate);
        // Preserve the fractional launch time. Rounding each gap would
        // drift away from the web schedule over the course of the cloud.
        next_time += spacing;
        Some((frame, frequency / rate as f32, phase))
    })
}

fn accumulate_grain(samples: &mut [f32], window: &[f32], step: f32, phase: f32) {
    const ANCHOR: usize = 256;
    let sin_step = sine(step);
    let half_sine = sine(0.5 * step);
    let cos_step = 1.0 - 2.0 * half_sine * half_sine;
    for (span, (out, window)) in samples
        .chunks_mut(ANCHOR)
        .zip(window.chunks(ANCHOR))
        .enumerate()
    {
        let phase = phase + (span * ANCHOR) as f32 * step;
        let (mut sin, mut cos) = (sine(phase), sine(phase + 0.25));
        for (out, &weight) in out.iter_mut().zip(window) {
            *out += weight * sin;
            (sin, cos) = (
                sin * cos_step + cos * sin_step,
                cos * cos_step - sin * sin_step,
            );
        }
    }
}

fn grain_cloud(
    layer: &LayerPatch,
    cached: &mut CachedLayer,
    rate: u32,
    waveform: &mut [f32],
    layer_cache: &mut [f32],
    tables: Option<Lookup<'_>>,
    _timing: &mut GrainTiming,
) -> Option<()> {
    let [_, decay, _, _, size_ms, density, _, _, _, drive] = layer.controls;
    let (window, storage) =
        layer_cache.split_at_mut_checked(milliseconds(rate, size_ms as usize))?;
    let cached_frames = match cached.transient_shape {
        TransientShape::GrainCloud {
            size_ms: saved,
            raw_frames,
        } if saved == size_ms => raw_frames,
        _ => {
            let half_turn_per_frame = 500.0 / (rate as f32 * size_ms);
            for (frame, weight) in window.iter_mut().enumerate() {
                // sin(pi * progress)^2 is the Hann window.
                let height = sine(frame as f32 * half_turn_per_frame);
                *weight = height * height;
            }
            0
        }
    };
    // Body Decay and Drive are applied after the grain sum. A shorter
    // render can use the prefix of an already cached sum.
    let reuse = cached_frames >= waveform.len()
        && cached.patch.model == Model::GrainCloud
        && cached.patch.controls[0] == layer.controls[0]
        && cached.patch.controls[2..9] == layer.controls[2..9];
    let raw = storage.get_mut(..waveform.len());
    let reused = match raw {
        Some(raw) if reuse => Some(raw),
        available => {
            #[cfg(feature = "profiling")]
            let start = cpu_cycle_count();
            waveform.fill(0.0);
            for (start, step, phase) in grain_schedule(layer, rate) {
                let Some(samples) = waveform.get_mut(start..) else {
                    break;
                };
                if samples.is_empty() {
                    break;
                }
                accumulate_grain(samples, window, step, phase);
            }
            #[cfg(feature = "profiling")]
            {
                _timing.accumulation = Some(cpu_cycle_count().wrapping_sub(start));
            }
            let raw_frames = if let Some(raw) = available {
                raw.copy_from_slice(waveform);
                raw.len()
            } else {
                0
            };
            cached.transient_shape = TransientShape::GrainCloud {
                size_ms,
                raw_frames,
            };
            None
        }
    };
    #[cfg(feature = "profiling")]
    let start = cpu_cycle_count();

    let envelope = DecayEnvelope::new(decay_rate(decay, rate));
    let normalization = positive_pow((density * size_ms / 2000.0).max(1.0), -0.5);
    let shape = LayerShape::new(drive, rate, waveform.len(), tables);
    // The first grain's window supplies the attack in the original voice.
    let steady = waveform
        .len()
        .saturating_sub(milliseconds(rate, RELEASE_FADE_MS) + 1);
    if let Some(raw) = reused {
        shape_grains(
            waveform.iter_mut().zip(raw.iter().copied()),
            steady,
            normalization,
            envelope,
            &shape,
        );
    } else {
        shape_grains(
            waveform.iter_mut().map(|sample| {
                let raw = *sample;
                (sample, raw)
            }),
            steady,
            normalization,
            envelope,
            &shape,
        );
    }
    #[cfg(feature = "profiling")]
    {
        _timing.shaping = Some(cpu_cycle_count().wrapping_sub(start));
    }
    Some(())
}

fn shape_grains<'a>(
    samples: impl Iterator<Item = (&'a mut f32, f32)>,
    steady: usize,
    normalization: f32,
    mut envelope: DecayEnvelope,
    shape: &LayerShape<'_>,
) {
    let mut samples = samples.enumerate();
    for (frame, (sample, raw)) in samples.by_ref().take(steady) {
        *sample = shape.drive(raw * normalization * envelope.next::<256>(frame));
    }
    for (frame, (sample, raw)) in samples {
        let gain = ((shape.last - frame) as f32 * shape.fade).min(1.0);
        *sample = shape.drive(raw * normalization * envelope.next::<256>(frame)) * gain;
    }
}

fn sync_exp2(octaves: f64) -> f64 {
    // Hard Sync uses -0.21..=5 octaves here. Reduce to the nearest octave,
    // then evaluate a [5/5] Padé approximant of exp on [-ln(2)/2, ln(2)/2].
    let whole = (octaves + 0.5) as u32;
    let x = (octaves - f64::from(whole)) * core::f64::consts::LN_2;
    let squared = x * x;
    let even = 30_240.0 + squared * (3360.0 + 30.0 * squared);
    let odd = x * (15_120.0 + squared * (420.0 + squared));
    let scale = f64::from_bits(u64::from(1023 + whole) << 52);
    scale * ((even + odd) / (even - odd))
}

const SYNC_OVERSAMPLE: u32 = 4;

struct SyncVoice {
    leader: f64,
    follower: f64,
    low1: f32,
    low2: f32,
    tail_sin: f32,
    tail_cos: f32,
}

impl SyncVoice {
    fn anchor_tail(&mut self) {
        let phase = self.follower as f32;
        self.tail_sin = sine(phase);
        self.tail_cos = sine(phase + 0.25);
    }

    fn advance<const STEADY: bool>(
        &mut self,
        step: f64,
        ratio: f64,
        lowpass: f32,
        (sin_step, cos_step): (f32, f32),
    ) -> f32 {
        let follower_step = step * ratio;
        for _ in 0..SYNC_OVERSAMPLE {
            let sample = if STEADY {
                self.tail_sin
            } else {
                sine(self.follower as f32)
            };
            self.low1 += lowpass * (sample - self.low1);
            self.low2 += lowpass * (self.low1 - self.low2);
            self.leader += step;
            if self.leader >= 1.0 {
                // Advance the follower by the fraction of this substep
                // that elapsed after the leader reset it.
                self.leader -= 1.0;
                self.follower = self.leader * ratio;
                if STEADY {
                    self.anchor_tail();
                }
            } else {
                self.follower += follower_step;
                if STEADY {
                    (self.tail_sin, self.tail_cos) = (
                        self.tail_sin * cos_step + self.tail_cos * sin_step,
                        self.tail_cos * cos_step - self.tail_sin * sin_step,
                    );
                }
            }
        }
        self.low2
    }
}

fn hard_sync(layer: &LayerPatch, rate: u32, waveform: &mut [f32], tables: Option<Lookup<'_>>) {
    let [tune, decay, sweep, sweep_time, bend, bend_time, tail_ratio, drive, ..] = layer.controls;
    let oversampled_rate = rate * SYNC_OVERSAMPLE;
    let lowpass = filter_coefficient(12_000.0, oversampled_rate);
    let dc_tracking = filter_coefficient(12.0, rate);
    let steady_step = f64::from(tune) / f64::from(oversampled_rate);
    // Tiny pitch errors can move a hard reset across a substep boundary.
    // Step the pitch envelope and phases in f64 to keep those times stable.
    let mut sweep_octaves = f64::from(sweep) / 12.0;
    let sweep_ratio =
        sync_exp2(-2000.0 * core::f64::consts::LOG2_10 / (f64::from(sweep_time) * f64::from(rate)));
    let mut bend_envelope = (bend > 0.0).then(|| DecayEnvelope::new(decay_rate(bend_time, rate)));
    let mut body_envelope = DecayEnvelope::new(decay_rate(decay, rate));
    let steady_follower_step = (steady_step * f64::from(tail_ratio)) as f32;
    let sin_step = sine(steady_follower_step);
    let half_sine = sine(0.5 * steady_follower_step);
    let rotation = (sin_step, 1.0 - 2.0 * half_sine * half_sine);
    let mut voice = SyncVoice {
        leader: 0.0,
        follower: 0.0,
        low1: 0.0,
        low2: 0.0,
        tail_sin: 0.0,
        tail_cos: 1.0,
    };
    let mut was_steady = false;
    let mut dc = 0.0;

    render_layer(drive, rate, waveform, tables, |frame| {
        let step = if sweep_octaves > 0.0 {
            let multiplier = sync_exp2(sweep_octaves);
            sweep_octaves = if multiplier == 1.0 {
                0.0
            } else {
                sweep_octaves * sweep_ratio
            };
            steady_step * multiplier
        } else {
            steady_step
        };
        let ratio = bend_envelope.as_mut().map_or(tail_ratio, |envelope| {
            tail_ratio + bend * envelope.next::<256>(frame)
        });
        let steady = step == steady_step && ratio == tail_ratio;
        let low2 = if steady {
            // Anchor every 256 substeps as well as on every fractional reset.
            if !was_steady || frame % 64 == 0 {
                voice.anchor_tail();
            }
            voice.advance::<true>(steady_step, f64::from(tail_ratio), lowpass, rotation)
        } else {
            voice.advance::<false>(step, f64::from(ratio), lowpass, rotation)
        };
        was_steady = steady;
        dc += dc_tracking * (low2 - dc);
        (low2 - dc) * body_envelope.next::<256>(frame)
    });
}

fn bit_logic(layer: &LayerPatch, rate: u32, waveform: &mut [f32], tables: Option<Lookup<'_>>) {
    let [tune, decay, drive, sweep, sweep_time, ratio, clock_hz, bits, operation, tone] =
        layer.controls;
    let mask = (1 << bits as u32) - 1u32;
    let step = clock_hz / rate as f32;
    let coefficient = filter_coefficient(tone, rate);
    let dc_tracking = filter_coefficient(18.0, rate);
    let pitch_decay = decay_rate(sweep_time, rate);
    let sweep_octaves = sweep / 12.0;
    let steady_phase_step = tune / clock_hz;
    let mut sweeping = sweep_octaves > 0.0;
    let mut body_envelope = DecayEnvelope::new(decay_rate(decay, rate));
    let (mut clock, mut first_phase, mut second_phase) = (1.0, 0.0, 0.31);
    let (mut held, mut filtered, mut dc) = (0.0, 0.0, 0.0);
    render_layer(drive, rate, waveform, tables, |frame| {
        if clock >= 1.0 {
            clock -= 1.0;
            let first = (first_phase * (mask + 1) as f32) as u32;
            let second = (second_phase * (mask + 1) as f32) as u32;
            let combined = match operation as u8 {
                1 => first & second,
                2 => first | second,
                3 => first + second,
                _ => first ^ second,
            };
            held = 2.0 * (combined & mask) as f32 / mask as f32 - 1.0;
            let phase_step = if sweeping {
                let multiplier = exp2(sweep_octaves * exp2(frame as f32 * pitch_decay));
                // The sweep only decreases. Once it rounds to unity,
                // subsequent ticks use the same unswept increment.
                sweeping = multiplier != 1.0;
                tune * multiplier / clock_hz
            } else {
                steady_phase_step
            };
            first_phase = wrap(first_phase + phase_step);
            second_phase = wrap(second_phase + phase_step * ratio);
        }
        clock += step;
        filtered += coefficient * (held - filtered);
        dc += dc_tracking * (filtered - dc);
        (filtered - dc) * body_envelope.next::<256>(frame)
    });
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;

    fn reference_hard_sync(layer: &LayerPatch, rate: u32, waveform: &mut [f32]) {
        use core::f64::consts::{LN_10, TAU};

        // Direct transcription of the browser renderer. Use f64 phases,
        // exact exponentials, and the browser's shaping order.
        let [tune, body, sweep, sweep_time, bend, bend_time, tail, drive, ..] =
            layer.controls.map(f64::from);
        let rate = f64::from(rate);
        let lowpass = 1.0 - (-TAU * 12_000.0 / (4.0 * rate)).exp();
        let dc_tracking = 1.0 - (-TAU * 12.0 / rate).exp();
        let boost = 10.0f64.powf(drive / 20.0);
        let decay = |time: f64, ms: f64| (-2.0 * LN_10 * time / (ms * 0.001)).exp();
        let last = waveform.len() - 1;
        let (mut leader, mut follower) = (0.0, 0.0);
        let (mut low1, mut low2, mut dc) = (0.0, 0.0, 0.0);
        for (frame, sample) in waveform.iter_mut().enumerate() {
            let time = frame as f64 / rate;
            let step = tune * (sweep / 12.0 * decay(time, sweep_time)).exp2() / (4.0 * rate);
            let ratio = tail + bend * decay(time, bend_time);
            for _ in 0..4 {
                let wave = (TAU * follower).sin();
                low1 += lowpass * (wave - low1);
                low2 += lowpass * (low1 - low2);
                leader += step;
                if leader >= 1.0 {
                    leader -= 1.0;
                    follower = leader * ratio;
                } else {
                    follower += step * ratio;
                }
            }
            dc += dc_tracking * (low2 - dc);
            let raw = (low2 - dc) * (time / 0.0007).min(1.0) * decay(time, body);
            let shaped = raw + ((raw * boost).tanh() - raw) * (drive / 5.0).min(1.0);
            // At 48 kHz this is the browser's 240-sample release.
            *sample = (shaped * ((last - frame) as f64 / (rate * 0.005)).min(1.0)) as f32;
        }
    }

    #[test]
    fn hard_sync_matches_web_reference_with_and_without_sweep_and_bend() {
        let mut table_memory = [0.0; lookup::BYTES as usize / 4];
        let tables = Some(lookup::test_tables(&mut table_memory));
        for rate in [32_000, 48_000, 96_000, 192_000] {
            for controls in [
                [51.0, 410.0, 21.0, 34.0, 5.5, 110.0, 1.0, 5.0, 0.0, 0.0],
                [25.0, 1000.0, 0.0, 34.0, 0.0, 110.0, 2.35, 0.0, 0.0, 0.0],
                [
                    120.0, 1000.0, 60.0, 200.0, 12.0, 500.0, 5.85, 24.0, 0.0, 0.0,
                ],
                [75.0, 45.0, 0.0, 1.0, 12.0, 5.0, 6.0, 0.0, 0.0, 0.0],
                [42.0, 200.0, 60.0, 1.0, 0.0, 5.0, 1.75, 12.0, 0.0, 0.0],
            ] {
                let layer = LayerPatch {
                    model: Model::HardSync,
                    controls,
                };
                let frames = milliseconds(rate, controls[1] as usize * 3 + 10)
                    .div_ceil(2)
                    .min(max_frames(rate));
                let mut reference = vec![0.0; frames];
                let mut waveform = vec![0.0; frames];
                reference_hard_sync(&layer, rate, &mut reference);
                hard_sync(&layer, rate, &mut waveform, tables);
                let mut peak = 0.0f32;
                let mut error_power = 0.0;
                let mut reference_power = 0.0;
                for (&actual, expected) in waveform.iter().zip(reference) {
                    let error = actual - expected;
                    peak = peak.max(error.abs());
                    error_power += f64::from(error).powi(2);
                    reference_power += f64::from(expected).powi(2);
                }
                let relative_rms = (error_power / reference_power).sqrt();
                assert!(
                    peak < 0.007 && relative_rms < 0.001,
                    "{rate} Hz, {controls:?}: peak error {peak}, relative RMS error {relative_rms}"
                );
                assert_eq!(waveform[0], 0.0);
                assert_eq!(waveform[frames - 1], 0.0);
            }
        }
    }

    #[test]
    fn grain_cloud_matches_web_onsets_and_waveform_samples() {
        // Generated by KickLab.html at 48 kHz, with the NT's one-second cap.
        let fixtures = [
            (
                [55.0, 470.0, 20.0, 70.0, 29.0, 107.0, 0.0, 4.8, 35.0, 4.0],
                34_080,
                76,
                [
                    0, 469, 920, 1405, 1832, 2222, 2700, 3099, 3501, 3994, 4485, 4943,
                ],
                [
                    (1, 3.95997e-7),
                    (239, 0.19716132),
                    (463, 0.5914804),
                    (1848, -0.8208714),
                    (11_360, 0.05108446),
                    (33_959, -0.00011483123),
                ],
            ),
            (
                [
                    120.0, 1000.0, 60.0, 200.0, 100.0, 220.0, 3.0, 24.0, 100.0, 0.0,
                ],
                48_000,
                236,
                [0, 155, 294, 464, 582, 668, 830, 924, 1021, 1196, 1371, 1516],
                [
                    (1, 1.2202182e-7),
                    (239, 0.0006868352),
                    (463, -0.028539622),
                    (1848, 0.05120612),
                    (16_000, -0.0783687),
                    (47_879, 0.004344283),
                ],
            ),
        ];
        let rate = 48_000;
        let mut table_memory = [0.0; lookup::BYTES as usize / 4];
        let tables = Some(lookup::test_tables(&mut table_memory));
        for (controls, frames, grains, onsets, samples) in fixtures {
            let patch = LayerPatch {
                model: Model::GrainCloud,
                controls,
            };
            let scheduled: std::vec::Vec<_> = grain_schedule(&patch, rate)
                .map(|(start, _, _)| start)
                .take_while(|&start| start < frames)
                .collect();
            assert_eq!(&scheduled[..onsets.len()], onsets);
            assert_eq!(scheduled.len(), grains);
            let mut waveform = vec![0.0; frames];
            let mut cached = CachedLayer::EMPTY;
            let mut buffer = vec![0.0; layer_cache_frames(rate)];
            grain_cloud(
                &patch,
                &mut cached,
                rate,
                &mut waveform,
                &mut buffer,
                tables,
                &mut GrainTiming::default(),
            )
            .unwrap();
            for (frame, expected) in samples {
                let error = (waveform[frame] - expected).abs();
                assert!(
                    error < 0.0002,
                    "frame {frame}: sample {}, expected {expected}",
                    waveform[frame]
                );
            }
            assert_eq!(waveform[0], 0.0);
            assert!(waveform[1].abs() > 0.0); // No additional attack ramp.
            assert_eq!(waveform[frames - 1], 0.0);
        }
    }

    #[test]
    fn short_grain_cache_reuses_only_the_matching_raw_cloud() {
        let rate = 48_000;
        let mut patch = LayerPatch {
            model: Model::GrainCloud,
            controls: crate::GRAIN_CLOUD.map(|control| control.decode(control.initial)),
        };
        let mut cache = CachedLayer::EMPTY;
        let mut storage = vec![f32::NAN; layer_cache_frames(rate)];
        let mut fresh_storage = storage.clone();
        for (frames, control, value, _reuse) in [
            (9000, 1, 120.0, false),
            (9000, 9, 10.0, true),
            (6000, 1, 80.0, true),
            (8500, 1, 110.0, true),
            (48_000, 1, 1000.0, false),
            (9000, 1, 120.0, false),
            (9000, 0, 70.0, false),
            (9000, 9, 5.0, true),
        ] {
            patch.controls[control] = value;
            let mut hit = vec![0.0; frames];
            let mut fresh = hit.clone();
            let mut timing = GrainTiming::default();
            grain_cloud(
                &patch,
                &mut cache,
                rate,
                &mut hit,
                &mut storage,
                None,
                &mut timing,
            )
            .unwrap();
            #[cfg(feature = "profiling")]
            assert_eq!(timing.accumulation.is_none(), _reuse);
            cache.patch = patch;
            let mut cold = CachedLayer::EMPTY;
            grain_cloud(
                &patch,
                &mut cold,
                rate,
                &mut fresh,
                &mut fresh_storage,
                None,
                &mut GrainTiming::default(),
            )
            .unwrap();
            assert_eq!(hit, fresh);
        }
    }

    #[test]
    fn grain_cloud_window_cache_matches_cold_renders_at_supported_rates() {
        let mut patch = LayerPatch {
            model: Model::GrainCloud,
            controls: crate::GRAIN_CLOUD.map(|control| control.decode(control.initial)),
        };
        for rate in [32_000, 44_100, 48_000, 96_000, 192_000] {
            let mut cached = CachedLayer::EMPTY;
            let mut buffer = vec![0.0; layer_cache_frames(rate)];
            let mut fresh_buffer = buffer.clone();
            let mut waveform = vec![f32::NAN; rate as usize / 10];
            let mut fresh_waveform = waveform.clone();
            for (size, density) in [(100.0, 220.0), (100.0, 18.0), (5.0, 105.0)] {
                patch.controls[4] = size;
                patch.controls[5] = density;
                let mut fresh = CachedLayer::EMPTY;
                grain_cloud(
                    &patch,
                    &mut fresh,
                    rate,
                    &mut fresh_waveform,
                    &mut fresh_buffer,
                    None,
                    &mut GrainTiming::default(),
                )
                .unwrap();
                grain_cloud(
                    &patch,
                    &mut cached,
                    rate,
                    &mut waveform,
                    &mut buffer,
                    None,
                    &mut GrainTiming::default(),
                )
                .unwrap();
                assert_eq!(waveform, fresh_waveform);
                assert!(waveform.iter().all(|sample| sample.is_finite()));
            }
        }
    }

    #[test]
    fn optimized_bit_logic_preserves_waveform() {
        let mut table_memory = [0.0; lookup::BYTES as usize / 4];
        let tables = Some(lookup::test_tables(&mut table_memory));
        for rate in [32_000, 48_000, 96_000, 192_000] {
            for controls in [
                [48.0, 370.0, 3.0, 25.0, 32.0, 2.05, 3560.0, 5.0, 0.0, 1600.0],
                [41.7, 298.0, 22.2, 8.0, 21.0, 1.33, 4479.0, 5.0, 3.0, 5179.0],
                [
                    75.0, 1000.0, 24.0, 60.0, 200.0, 6.0, 15000.0, 12.0, 1.0, 8500.0,
                ],
                [25.0, 1000.0, 0.0, 0.0, 1.0, 1.0, 500.0, 3.0, 2.0, 160.0],
            ] {
                let layer = LayerPatch {
                    model: Model::BitLogic,
                    controls,
                };
                let frames = milliseconds(rate, controls[1] as usize * 3 + 10)
                    .div_ceil(2)
                    .min(max_frames(rate));
                let mut reference = vec![0.0; frames];
                let mut optimized = vec![0.0; frames];
                reference_bit_logic(&layer, rate, &mut reference, tables);
                bit_logic(&layer, rate, &mut optimized, tables);
                let mut peak = 0.0f32;
                let mut error_power = 0.0f64;
                let mut reference_power = 0.0f64;
                for (actual, expected) in optimized.into_iter().zip(reference) {
                    let error = actual - expected;
                    peak = peak.max(error.abs());
                    error_power += f64::from(error).powi(2);
                    reference_power += f64::from(expected).powi(2);
                }
                let relative_rms = (error_power / reference_power).sqrt();
                assert!(
                    peak < 0.001 && relative_rms < 0.0003,
                    "{rate} Hz, {controls:?}: peak error {peak}, relative RMS error {relative_rms}"
                );
            }
        }
    }

    fn reference_bit_logic(
        layer: &LayerPatch,
        rate: u32,
        waveform: &mut [f32],
        tables: Option<Lookup<'_>>,
    ) {
        let [tune, decay, _, sweep, sweep_time, ratio, clock_hz, bits, operation, tone] =
            layer.controls;
        let mask = (1 << bits as u32) - 1u32;
        let step = clock_hz / rate as f32;
        let coefficient = filter_coefficient(tone, rate);
        let dc_tracking = filter_coefficient(18.0, rate);
        let pitch_decay = decay_rate(sweep_time, rate);
        let body_decay = decay_rate(decay, rate);
        let (mut clock, mut first_phase, mut second_phase) = (1.0, 0.0, 0.31);
        let (mut held, mut filtered, mut dc) = (0.0, 0.0, 0.0);
        let shape = LayerShape::new(layer.controls[2], rate, waveform.len(), tables);
        for (frame, sample) in waveform.iter_mut().enumerate() {
            if clock >= 1.0 {
                clock -= 1.0;
                let first = (first_phase * (mask + 1) as f32) as u32;
                let second = (second_phase * (mask + 1) as f32) as u32;
                let combined = match operation as u8 {
                    1 => first & second,
                    2 => first | second,
                    3 => first + second,
                    _ => first ^ second,
                };
                held = 2.0 * (combined & mask) as f32 / mask as f32 - 1.0;
                let phase_step =
                    tune * exp2(sweep / 12.0 * exp2(frame as f32 * pitch_decay)) / clock_hz;
                first_phase = wrap(first_phase + phase_step);
                second_phase = wrap(second_phase + phase_step * ratio);
            }
            clock += step;
            filtered += coefficient * (held - filtered);
            dc += dc_tracking * (filtered - dc);
            *sample = shape.sample(frame, (filtered - dc) * exp2(frame as f32 * body_decay));
        }
    }

    #[test]
    fn sine_matches_source_phase_and_polarity() {
        for index in 0..1000 {
            let turns = index as f32 / 311.0;
            assert!((sine(turns) - (TAU * turns).sin()).abs() < 0.00001);
        }
    }

    fn reference_modal_drum(
        layer: &LayerPatch,
        rate: u32,
        waveform: &mut [f32],
        tables: Option<Lookup<'_>>,
    ) {
        let [_, _, _, _, _, _, beater, hardness, ..] = layer.controls;
        let modes = modal_modes(layer, rate);
        let mut noise = NoiseBand::new(650.0, 1200.0 + hardness * 72.0, rate);
        let beater_decay = decay_rate(13.0 - hardness * 0.1, rate);
        render_layer(layer.controls[2], rate, waveform, tables, |frame| {
            let frame = frame as f32;
            let body = modes
                .iter()
                .map(|&(phase, damping, level)| sine(phase * frame) * exp2(damping * frame) * level)
                .sum::<f32>();
            body + noise.next() * exp2(frame * beater_decay) * beater / 70.0
        });
    }

    #[test]
    fn optimized_modal_drum_preserves_waveform() {
        let mut table_memory = [0.0; lookup::BYTES as usize / 4];
        let tables = Some(lookup::test_tables(&mut table_memory));
        for rate in [32_000, 48_000, 96_000, 192_000] {
            for controls in [
                [25.0, 45.0, 0.0, 0.0, 100.0, 50.0, 0.0, 0.0, 0.0, 0.0],
                [57.0, 350.0, 1.0, 62.0, 44.0, 50.0, 60.0, 45.0, 0.0, 0.0],
                [102.0, 915.0, 17.2, 58.0, 11.0, 34.0, 45.0, 73.0, 0.0, 0.0],
                [120.0, 1000.0, 24.0, 100.0, 0.0, 100.0, 100.0, 0.0, 0.0, 0.0],
                [25.0, 1000.0, 0.0, 100.0, 100.0, 0.0, 100.0, 100.0, 0.0, 0.0],
            ] {
                let layer = LayerPatch {
                    model: Model::ModalDrum,
                    controls,
                };
                let frames = milliseconds(rate, controls[1] as usize * 3 + 10)
                    .div_ceil(2)
                    .min(max_frames(rate));
                let mut reference = vec![0.0; frames];
                let mut optimized = vec![0.0; frames];
                reference_modal_drum(&layer, rate, &mut reference, tables);
                let mut cache = CachedLayer::EMPTY;
                let mut layer_cache = vec![0.0; layer_cache_frames(rate)];
                modal_drum(
                    &layer,
                    &mut cache,
                    rate,
                    &mut optimized,
                    &mut layer_cache,
                    tables,
                )
                .unwrap();
                let mut peak = 0.0f32;
                let mut error_power = 0.0f64;
                let mut reference_power = 0.0f64;
                for (actual, expected) in optimized.into_iter().zip(reference) {
                    let error = actual - expected;
                    peak = peak.max(error.abs());
                    error_power += f64::from(error).powi(2);
                    reference_power += f64::from(expected).powi(2);
                }
                let relative_rms = (error_power / reference_power).sqrt();
                assert!(
                    peak < 0.001 && relative_rms < 0.0003,
                    "{rate} Hz, {controls:?}: peak error {peak}, relative RMS error {relative_rms}"
                );
            }
        }
    }

    #[test]
    fn modal_beater_edits_match_full_renders_and_leave_the_tail_alone() {
        let rate = 48_000;
        let mut table_memory = [0.0; lookup::BYTES as usize / 4];
        let tables = Some(lookup::test_tables(&mut table_memory));
        let mut layer = LayerPatch {
            model: Model::ModalDrum,
            controls: [102.0, 915.0, 17.2, 58.0, 11.0, 34.0, 45.0, 73.0, 0.0, 0.0],
        };
        let mut waveform = vec![0.0; max_frames(rate)];
        let mut layer_cache = vec![0.0; layer_cache_frames(rate)];
        let mut expected = waveform.clone();
        let mut cold_cache = layer_cache.clone();
        let mut cached = CachedLayer::EMPTY;
        modal_drum(
            &layer,
            &mut cached,
            rate,
            &mut waveform,
            &mut layer_cache,
            tables,
        )
        .unwrap();
        cached.patch = layer;
        let original_tail = waveform[max_beater_frames(rate)..].to_vec();
        for (level, hardness) in [
            (85.0, 73.0),
            (85.0, 100.0),
            (0.0, 0.0),
            (100.0, 0.0),
            (40.0, 50.0),
        ] {
            layer.controls[6] = level;
            layer.controls[7] = hardness;
            modal_drum(
                &layer,
                &mut cached,
                rate,
                &mut waveform,
                &mut layer_cache,
                tables,
            )
            .unwrap();
            cached.patch = layer;
            let mut cold = CachedLayer::EMPTY;
            modal_drum(
                &layer,
                &mut cold,
                rate,
                &mut expected,
                &mut cold_cache,
                tables,
            )
            .unwrap();
            assert_eq!(waveform, expected);
            assert_eq!(waveform[max_beater_frames(rate)..], original_tail);
        }
    }

    fn reference_pitch_sweep(layer: &LayerPatch, rate: u32, waveform: &mut [f32]) {
        let [tune, decay, _, sweep, sweep_time, curve, click, click_tone, click_decay, _] =
            layer.controls;
        let mut beater = NoiseBand::new(900.0, click_tone, rate);
        let curvature = exp2(curve / 50.0);
        let sweep_frames = sweep_time * rate as f32 / 1000.0;
        let pitch_decay = decay_rate(sweep_time, rate);
        let body_decay = decay_rate(decay, rate);
        let click_decay = decay_rate(click_decay, rate);
        let mut phase = 0.0;
        render_layer(layer.controls[2], rate, waveform, None, |frame| {
            let frame = frame as f32;
            let body = sine(phase) * exp2(frame * body_decay);
            let position = if curve == 0.0 {
                frame
            } else {
                sweep_frames * positive_pow(frame / sweep_frames, curvature)
            };
            phase = wrap(
                phase + tune * exp2(sweep / 12.0 * exp2(position * pitch_decay)) / rate as f32,
            );
            body + beater.next() * exp2(frame * click_decay) * click / 100.0
        });
    }

    #[test]
    fn optimized_pitch_sweep_preserves_waveform() {
        let mut memory = [0.0; lookup::BYTES as usize / 4];
        let tables = Some(lookup::test_tables(&mut memory));
        let patches = [
            [50.0, 460.0, 2.0, 24.0, 24.0, 0.0, 24.0, 5500.0, 6.0, 0.0],
            [
                50.0, 1301.0, 12.9, 27.0, 101.0, -20.0, 42.0, 2901.0, 14.3, 0.0,
            ],
            [25.0, 45.0, 0.0, 0.0, 1.0, -100.0, 0.0, 900.0, 1.0, 0.0],
        ];
        for rate in [32_000, 48_000, 96_000, 192_000] {
            for controls in
                patches
                    .into_iter()
                    .chain(
                        [-100.0, -35.0, 0.0, 35.0, 100.0]
                            .into_iter()
                            .flat_map(|curve| {
                                [1.0, 67.0, 200.0].into_iter().map(move |time| {
                                    [
                                        120.0, 2000.0, 24.0, 60.0, time, curve, 100.0, 10_000.0,
                                        45.0, 0.0,
                                    ]
                                })
                            }),
                    )
            {
                let patch = LayerPatch {
                    model: Model::PitchSweep,
                    controls,
                };
                let end = milliseconds(rate, controls[1] as usize * 3 + 10).div_ceil(2);
                let mut reference = vec![0.0; end];
                let mut optimized = vec![0.0; end];
                reference_pitch_sweep(&patch, rate, &mut reference);
                let mut components = vec![0.0; 2 * max_click_frames(rate)];
                let mut cache = CachedLayer::EMPTY;
                pitch_sweep(
                    &patch,
                    &mut cache,
                    rate,
                    &mut optimized,
                    &mut components,
                    tables,
                )
                .unwrap();
                let (mut peak, mut error_power, mut reference_power) = (0.0f32, 0.0f64, 0.0f64);
                for (actual, expected) in optimized.into_iter().zip(reference) {
                    let error = actual - expected;
                    peak = peak.max(error.abs());
                    error_power += f64::from(error).powi(2);
                    reference_power += f64::from(expected).powi(2);
                }
                let relative_rms = (error_power / reference_power).sqrt();
                assert!(
                    peak < 0.01 && relative_rms < 0.003,
                    "{rate} Hz, {controls:?}: peak error {peak}, relative RMS error {relative_rms}"
                );
            }
        }
    }

    #[test]
    fn click_edits_match_a_full_render_and_preserve_samples_outside_the_click() {
        let rate = 48_000;
        let mut table_memory = [0.0; lookup::BYTES as usize / 4];
        let tables = Some(lookup::test_tables(&mut table_memory));
        let mut patch = LayerPatch {
            model: Model::PitchSweep,
            controls: [
                55.0, 460.0, 12.9, 27.0, 101.0, -20.0, 42.0, 2901.0, 45.0, 0.0,
            ],
        };
        let end = milliseconds(rate, 460 * 3 + 10).div_ceil(2);
        let mut cached_wave = vec![0.0; end];
        let mut fresh_wave = vec![0.0; end];
        let mut components = vec![0.0; 2 * max_click_frames(rate)];
        let mut fresh_components = components.clone();
        let mut cached = CachedLayer::EMPTY;
        pitch_sweep(
            &patch,
            &mut cached,
            rate,
            &mut cached_wave,
            &mut components,
            tables,
        )
        .unwrap();
        cached.patch = patch;

        for (index, value) in [
            (6, 12.0),
            (7, 8400.0),
            (8, 2.0),
            (6, 0.0),
            (8, 24.0),
            (6, 80.0),
        ] {
            let previous = patch;
            patch.controls[index] = value;
            // A marker beyond both click spans detects any write into the tail.
            let untouched = click_frames(&previous, rate).max(click_frames(&patch, rate)) + 1;
            let marker = cached_wave[untouched];
            cached_wave[untouched] = 2.0;
            pitch_sweep(
                &patch,
                &mut cached,
                rate,
                &mut cached_wave,
                &mut components,
                tables,
            )
            .unwrap();
            assert_eq!(cached_wave[untouched], 2.0);
            cached_wave[untouched] = marker;
            cached.patch = patch;
            let mut cold = CachedLayer::EMPTY;
            pitch_sweep(
                &patch,
                &mut cold,
                rate,
                &mut fresh_wave,
                &mut fresh_components,
                tables,
            )
            .unwrap();
            assert_eq!(cached_wave, fresh_wave);
        }

        // A body or Drive edit invalidates the shaped tail as well.
        for (index, value) in [(0, 82.0), (2, 0.7)] {
            patch.controls[index] = value;
            pitch_sweep(
                &patch,
                &mut cached,
                rate,
                &mut cached_wave,
                &mut components,
                tables,
            )
            .unwrap();
            cached.patch = patch;
            let mut cold = CachedLayer::EMPTY;
            pitch_sweep(
                &patch,
                &mut cold,
                rate,
                &mut fresh_wave,
                &mut fresh_components,
                tables,
            )
            .unwrap();
            assert_eq!(cached_wave, fresh_wave);
        }
    }
}
