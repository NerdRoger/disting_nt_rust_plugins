//! RMS-based mix attenuation with held peak protection. No gain above unity.
#![forbid(unsafe_code)]

// Small preallocated chunks avoid a maximum firmware-block-size assumption.
pub const CHUNK: usize = 16;
// Peak headroom is independent of the user's loudness setting. This is the
// oscillator's nominal full-scale swing, not a claim about the final bus sum.
pub const PEAK_CEILING: f32 = 5.0;

pub const LEVEL_MIN: i16 = -60; // NT shows the minimum as -inf; DSP treats it as mute.
pub const LEVEL_DEFAULT: i16 = -6;
pub const DEFAULT_GAIN: f32 = 0.501_187_2;

/// Cache dB conversion across audio blocks, including while Level is modulated.
#[derive(Clone, Copy)]
pub struct Level {
    db: i16,
    amplitude: f32,
}

impl Level {
    pub const fn new() -> Self {
        Self {
            db: LEVEL_DEFAULT,
            amplitude: DEFAULT_GAIN,
        }
    }

    pub fn amplitude(&mut self, db: i16) -> f32 {
        let db = db.clamp(LEVEL_MIN, 0);
        if db != self.db {
            self.db = db;
            self.amplitude = if db == LEVEL_MIN {
                0.0
            } else {
                distingnt_api::api::math::exp2(db as f32 * (core::f32::consts::LOG2_10 / 20.0))
            };
        }
        self.amplitude
    }
}

#[derive(Clone, Copy)]
pub struct LevelControl {
    mix_power: f32,
    reference_power: f32,
    gain: f32,
    peak_gain: f32,
    peak_hold: usize,
    attack: f32,
    recovery: f32,
    hold_frames: usize,
    prime_gain: bool,
}

impl LevelControl {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1) as f32;
        Self {
            mix_power: 0.0,
            reference_power: 0.0,
            gain: 1.0,
            peak_gain: 1.0,
            peak_hold: 0,
            attack: 1.0 / (0.05 * rate),
            recovery: 1.0 / (1.0 * rate),
            hold_frames: (sample_rate / 50) as usize,
            prime_gain: false,
        }
    }

    pub fn reset(&mut self) {
        self.mix_power = 0.0;
        self.reference_power = 0.0;
        self.gain = 1.0;
        self.peak_gain = 1.0;
        self.peak_hold = 0;
        self.prime_gain = false;
    }

    /// Mode changes restart the detector; acquire attenuation from the first
    /// audible chunk instead of starting at unity and producing a loud burst.
    pub fn restart_for_mode_change(&mut self) {
        self.reset();
        self.prime_gain = true;
    }

    /// `voice_power` is the sum of each voice's squared samples, before mixing.
    /// `voice_weight` is the mean sum of squared MIDI gates (unity gates in CV).
    /// A barely-open voice therefore contributes barely any normalization weight.
    pub fn process(&mut self, mix: &mut [f32], voice_power: f32, voice_weight: f32, ceiling: f32) {
        self.process_channels(mix, &mut [], voice_power, voice_weight, ceiling, false);
    }

    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        voice_power: f32,
        voice_weight: f32,
        ceiling: f32,
    ) {
        self.process_channels(left, right, voice_power, voice_weight, ceiling, true);
    }

    #[inline(never)]
    fn process_channels(
        &mut self,
        mix: &mut [f32],
        right: &mut [f32],
        voice_power: f32,
        voice_weight: f32,
        ceiling: f32,
        stereo: bool,
    ) {
        if stereo && mix.len() != right.len() {
            mix.fill(0.0);
            right.fill(0.0);
            self.reset();
            return;
        }
        if mix.is_empty() {
            return;
        }
        if voice_weight <= 0.0 || ceiling <= 0.0 {
            mix.fill(0.0);
            if stereo {
                right.fill(0.0);
            }
            self.reset();
            return;
        }
        let frames = mix.len();
        let (mut left_power, mut right_power) = (0.0, 0.0);
        let (mut left_peak, mut right_peak) = (0.0f32, 0.0f32);
        if stereo {
            for (&left, &right) in mix.iter().zip(right.iter()) {
                left_power += left * left;
                right_power += right * right;
                left_peak = left_peak.max(left.abs());
                right_peak = right_peak.max(right.abs());
            }
        } else {
            for &sample in mix.iter() {
                left_power += sample * sample;
                left_peak = left_peak.max(sample.abs());
            }
        }
        let total_power = if stereo {
            left_power + right_power
        } else {
            left_power
        };
        let peak = if stereo {
            left_peak.max(right_peak)
        } else {
            left_peak
        };
        let power = total_power / frames as f32;
        if power <= 0.000000000001 {
            // Hold both detectors and gain through silence/cancellation instead
            // of letting stale detector history move the gain without sound.
            let gain = self.gain.min(self.peak_gain);
            for sample in mix {
                *sample *= gain;
            }
            if stereo {
                for sample in right {
                    *sample *= gain;
                }
            }
            return;
        }
        // The floor retains a lone note's natural attack/release. With multiple
        // notes, a new attack contributes progressively instead of halving the
        // reference as soon as Note On arrives.
        let reference = voice_power / (frames as f32 * voice_weight.max(1.0));
        // Identical detector timing on both sides makes a fading chord's gain
        // independent of its amplitude envelope. Never chase individual cycles.
        let detector = (self.attack * frames as f32).min(1.0);
        self.mix_power += detector * (power - self.mix_power);
        self.reference_power += detector * (reference - self.reference_power);
        let target = if self.mix_power > 0.0000000001 {
            sqrt((self.reference_power / self.mix_power).clamp(0.0, 1.0))
        } else {
            self.gain // Silence must not cause gain recovery or noise amplification.
        };
        if self.prime_gain {
            self.gain = target;
            self.prime_gain = false;
        }
        let speed = if target < self.gain {
            self.attack
        } else {
            self.recovery
        };
        let next_gain = self.gain + (speed * frames as f32).min(1.0) * (target - self.gain);
        let safe_gain = if peak > ceiling { ceiling / peak } else { 1.0 };
        // Analyze the complete small chunk before emitting it. A held ceiling
        // avoids releasing on every waveform trough; peak recovery takes 200 ms.
        if safe_gain < self.peak_gain {
            self.peak_gain = safe_gain;
            self.peak_hold = self.hold_frames;
        } else if self.peak_hold > 0 {
            self.peak_hold = self.peak_hold.saturating_sub(frames);
        } else {
            self.peak_gain +=
                (5.0 * self.recovery * frames as f32).min(1.0) * (safe_gain - self.peak_gain);
        }
        let start = self.gain;
        let gain_step = (next_gain - start) / frames as f32;
        if stereo {
            for (i, (left, right)) in mix.iter_mut().zip(right.iter_mut()).enumerate() {
                let gain = (start + gain_step * (i + 1) as f32).min(self.peak_gain);
                *left *= gain;
                *right *= gain;
            }
        } else {
            for (i, sample) in mix.iter_mut().enumerate() {
                let gain = start + gain_step * (i + 1) as f32;
                *sample *= gain.min(self.peak_gain);
            }
        }
        self.gain = next_gain;
    }
    #[cfg(test)]
    fn process_channels_reference<const STEREO: bool>(
        &mut self,
        mix: &mut [f32],
        right: &mut [f32],
        voice_power: f32,
        voice_weight: f32,
        ceiling: f32,
    ) {
        if STEREO && mix.len() != right.len() {
            mix.fill(0.0);
            right.fill(0.0);
            self.reset();
            return;
        }
        if mix.is_empty() {
            return;
        }
        if voice_weight <= 0.0 || ceiling <= 0.0 {
            mix.fill(0.0);
            if STEREO {
                right.fill(0.0);
            }
            self.reset();
            return;
        }
        let frames = mix.len();
        let mut total_power = mix.iter().map(|v| v * v).sum::<f32>();
        if STEREO {
            total_power += right.iter().map(|v| v * v).sum::<f32>();
        }
        let power = total_power / frames as f32;
        if power <= 0.000000000001 {
            // Hold both detectors and gain through silence/cancellation instead
            // of letting stale detector history move the gain without sound.
            let gain = self.gain.min(self.peak_gain);
            for sample in mix {
                *sample *= gain;
            }
            if STEREO {
                for sample in right {
                    *sample *= gain;
                }
            }
            return;
        }
        // The floor retains a lone note's natural attack/release. With multiple
        // notes, a new attack contributes progressively instead of halving the
        // reference as soon as Note On arrives.
        let reference = voice_power / (frames as f32 * voice_weight.max(1.0));
        // Identical detector timing on both sides makes a fading chord's gain
        // independent of its amplitude envelope. Never chase individual cycles.
        let detector = (self.attack * frames as f32).min(1.0);
        self.mix_power += detector * (power - self.mix_power);
        self.reference_power += detector * (reference - self.reference_power);
        let target = if self.mix_power > 0.0000000001 {
            sqrt((self.reference_power / self.mix_power).clamp(0.0, 1.0))
        } else {
            self.gain // Silence must not cause gain recovery or noise amplification.
        };
        if self.prime_gain {
            self.gain = target;
            self.prime_gain = false;
        }
        let speed = if target < self.gain {
            self.attack
        } else {
            self.recovery
        };
        let next_gain = self.gain + (speed * frames as f32).min(1.0) * (target - self.gain);
        let mut peak = mix.iter().fold(0.0f32, |peak, v| peak.max(v.abs()));
        if STEREO {
            peak = right.iter().fold(peak, |peak, v| peak.max(v.abs()));
        }
        let safe_gain = if peak > ceiling { ceiling / peak } else { 1.0 };
        // Analyze the complete small chunk before emitting it. A held ceiling
        // avoids releasing on every waveform trough; peak recovery takes 200 ms.
        if safe_gain < self.peak_gain {
            self.peak_gain = safe_gain;
            self.peak_hold = self.hold_frames;
        } else if self.peak_hold > 0 {
            self.peak_hold = self.peak_hold.saturating_sub(frames);
        } else {
            self.peak_gain +=
                (5.0 * self.recovery * frames as f32).min(1.0) * (safe_gain - self.peak_gain);
        }
        let start = self.gain;
        let gain_step = (next_gain - start) / frames as f32;
        for (i, sample) in mix.iter_mut().enumerate() {
            let gain = start + gain_step * (i + 1) as f32;
            *sample *= gain.min(self.peak_gain);
        }
        if STEREO {
            for (i, sample) in right.iter_mut().enumerate() {
                let gain = start + gain_step * (i + 1) as f32;
                *sample *= gain.min(self.peak_gain);
            }
        }
        self.gain = next_gain;
    }
}

/// A short mode-switch crossfade from the last emitted sample to the new mix.
/// This runs after normalization, so the detector sees the real incoming audio.
#[derive(Clone, Copy)]
pub struct ModeTransition {
    last: f32,
    from: f32,
    remaining: usize,
    length: usize,
    rendered: bool,
}

impl ModeTransition {
    pub fn new(rate: u32) -> Self {
        Self {
            last: 0.0,
            from: 0.0,
            remaining: 0,
            length: (rate as usize / 50).max(1),
            rendered: false,
        }
    }

    pub fn start(&mut self) {
        if self.rendered {
            self.from = self.last;
            self.remaining = self.length;
        }
    }

    pub fn current(&self) -> f32 {
        self.last
    }

    pub fn start_from(&mut self, sample: f32) {
        self.last = sample;
        self.from = sample;
        self.remaining = self.length;
        self.rendered = true;
    }

    /// Adopt an already-connected destination without freezing its waveform.
    pub fn adopt(&mut self, sample: f32) {
        self.last = sample;
        self.from = sample;
        self.remaining = 0;
        self.rendered = true;
    }

    pub fn silence(&mut self) {
        self.last = 0.0;
        self.from = 0.0;
        self.remaining = 0;
        self.rendered = false;
    }

    pub fn apply(&mut self, mix: &mut [f32]) {
        if self.remaining > 0 {
            for sample in mix.iter_mut() {
                if self.remaining == 0 {
                    break;
                }
                let t = (self.length - self.remaining) as f32 / self.length as f32;
                *sample = self.from + (*sample - self.from) * t;
                self.remaining -= 1;
            }
        }
        if let Some(&last) = mix.last() {
            self.last = last;
            self.rendered = true;
        }
    }
}

// Positive finite input <= 1. Newton refinement avoids importing an unprovided
// sqrtf firmware symbol or a libm dependency; the bit estimate bounds iteration.
pub(super) fn sqrt(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    let mut root = f32::from_bits((value.to_bits() >> 1) + 0x1fc0_0000);
    for _ in 0..4 {
        root = 0.5 * (root + value / root);
    }
    root
}

#[cfg(test)]
#[path = "levels_tests.rs"]
mod tests;
