//! Direct segmented oscillator. Smooth=0 evaluates the drawn curve at each
//! audio sample; only explicit smoothing uses an FFT table. Edits crossfade
//! over five milliseconds without resetting phase.
#![forbid(unsafe_code)]

pub const SIZE: usize = 256;
pub const LEVELS: usize = 1;
pub const MAX_SEGMENTS: usize = 32;
#[cfg(test)]
const TWIDDLES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wave {
    pub count: usize,
    pub smoothing: i16,
    pub boundaries: [i16; MAX_SEGMENTS + 1],
    pub curves: [i16; MAX_SEGMENTS],
}

impl Wave {
    pub fn new(count: usize) -> Self {
        let count = count.clamp(2, MAX_SEGMENTS);
        let mut wave = Self {
            count,
            smoothing: 0,
            boundaries: [0; MAX_SEGMENTS + 1],
            curves: [0; MAX_SEGMENTS],
        };
        for i in 1..count {
            wave.boundaries[i] = default_level(i, count);
        }
        wave
    }

    pub fn sample(&self, phase: f32) -> f32 {
        if phase <= 0.0 || phase >= 1.0 {
            return self.boundary_level(0);
        }
        let position = phase * self.count as f32;
        let index = (position as usize).min(self.count - 1);
        self.segment_sample(index, position - index as f32)
    }

    pub fn boundary_level(&self, boundary: usize) -> f32 {
        let boundary = if boundary >= self.count { 0 } else { boundary };
        self.boundaries[boundary.min(MAX_SEGMENTS)].clamp(-100, 100) as f32 * 0.01
    }

    /// Both sides of every join read the same boundary value. The outer
    /// boundaries share the editable seam value stored at boundary zero.
    pub fn segment_sample(&self, index: usize, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        // Rational easing is continuous in curve, monotonic, bounded, and
        // exactly linear at zero. +/-100 bends by a factor of sixteen.
        let bend = self.curves[index.min(MAX_SEGMENTS - 1)].clamp(-100, 100) as f32 * 0.01;
        let weight = if bend >= 0.0 {
            t / (1.0 + 15.0 * bend * (1.0 - t))
        } else {
            1.0 - (1.0 - t) / (1.0 - 15.0 * bend * t)
        };
        let start = self.boundary_level(index);
        let end = self.boundary_level(index + 1);
        // Return the stored anchors exactly, avoiding interpolation roundoff
        // between two differently curved neighboring segments.
        if t == 0.0 {
            return start;
        }
        if t == 1.0 {
            return end;
        }
        start + (end - start) * weight
    }
}

// Add a harmonic without allocating another table or doing a second FFT.
fn add_harmonic(data: &mut [Complex; SIZE], twiddles: &[Complex; SIZE], k: usize, a: f32, b: f32) {
    let step = twiddles[k & (SIZE - 1)];
    let (mut cosine, mut sinusoid) = (1.0, 0.0);
    for value in data {
        value.re += a * cosine + b * sinusoid;
        let next = cosine * step.re + sinusoid * step.im;
        sinusoid = sinusoid * step.re - cosine * step.im;
        cosine = next;
    }
}

/// Explicit smoothing retains shared boundaries, including the seam. This is
/// independent of oscillator pitch; no automatic anti-alias filter is applied.
fn anchor_boundaries(wave: &Wave, data: &mut [Complex; SIZE], twiddles: &[Complex; SIZE]) {
    let n = wave.count.clamp(2, MAX_SEGMENTS);
    // A second projection compensates for linear table lookup between grid
    // samples when the segment count does not divide SIZE (e.g. 7 or 31).
    for _ in 0..2 {
        let mut errors = [0.0; MAX_SEGMENTS];
        for (i, error) in errors.iter_mut().take(n).enumerate() {
            let position = i as f32 * SIZE as f32 / n as f32;
            let index = position as usize & (SIZE - 1);
            let fraction = position - index as f32;
            let actual =
                data[index].re + (data[(index + 1) & (SIZE - 1)].re - data[index].re) * fraction;
            let target = wave.boundary_level(i);
            *error = target - actual;
        }
        let dc = errors.iter().take(n).sum::<f32>() / n as f32;
        for value in data.iter_mut() {
            value.re += dc;
        }
        for k in 1..=n / 2 {
            let angle = core::f32::consts::TAU * k as f32 / n as f32;
            let (wc, ws) = (sine(angle + core::f32::consts::FRAC_PI_2), sine(angle));
            let (mut cosine, mut sinusoid) = (1.0, 0.0);
            let (mut a, mut b) = (0.0, 0.0);
            for &error in errors.iter().take(n) {
                a += error * cosine;
                b += error * sinusoid;
                let next = cosine * wc - sinusoid * ws;
                sinusoid = sinusoid * wc + cosine * ws;
                cosine = next;
            }
            let scale = if n % 2 == 0 && k == n / 2 { 1.0 } else { 2.0 } / n as f32;
            add_harmonic(data, twiddles, k, a * scale, b * scale);
        }
    }
    // Projection roundoff only; no waveform-wide translation or normalization.
    data[0].re = wave.boundary_level(0);
}

pub fn default_level(node: usize, count: usize) -> i16 {
    // A bipolar triangle-like starting wave, including for two segments.
    let phase = node as f32 / count as f32;
    let value = if count == 2 {
        if node == 1 {
            1.0
        } else {
            0.0
        }
    } else if phase < 0.25 {
        phase * 4.0
    } else if phase < 0.75 {
        2.0 - phase * 4.0
    } else {
        phase * 4.0 - 4.0
    };
    (value * 100.0) as i16
}

#[derive(Clone, Copy, Default)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    fn mul(self, rhs: Self) -> Self {
        Self {
            re: self.re * rhs.re - self.im * rhs.im,
            im: self.re * rhs.im + self.im * rhs.re,
        }
    }
}

fn sine(mut x: f32) -> f32 {
    let pi = core::f32::consts::PI;
    if x > pi {
        x -= 2.0 * pi;
    }
    if x > pi * 0.5 {
        x = pi - x;
    }
    if x < -pi * 0.5 {
        x = -pi - x;
    }
    let q = x * x;
    x * (1.0
        + q * (-1.0 / 6.0
            + q * (1.0 / 120.0 + q * (-1.0 / 5040.0 + q * (1.0 / 362880.0 - q / 39916800.0)))))
}

fn fft(data: &mut [Complex; SIZE], twiddles: &[Complex; SIZE], inverse: bool) {
    for i in 0..SIZE {
        let j = (i.reverse_bits() >> (usize::BITS - 8)) & (SIZE - 1);
        if i < j {
            data.swap(i, j);
        }
    }
    let mut width = 2;
    while width <= SIZE {
        let half = width / 2;
        for base in (0..SIZE).step_by(width) {
            for k in 0..half {
                let mut w = twiddles[(k * SIZE / width) & (SIZE / 2 - 1)];
                if inverse {
                    w.im = -w.im;
                }
                let a = data[(base + k) & (SIZE - 1)];
                let b = data[(base + k + half) & (SIZE - 1)].mul(w);
                data[(base + k) & (SIZE - 1)] = Complex {
                    re: a.re + b.re,
                    im: a.im + b.im,
                };
                data[(base + k + half) & (SIZE - 1)] = Complex {
                    re: a.re - b.re,
                    im: a.im - b.im,
                };
            }
        }
        width *= 2;
    }
    if inverse {
        for value in data {
            value.re /= SIZE as f32;
            value.im /= SIZE as f32;
        }
    }
}

#[derive(Clone, Copy)]
pub struct Block {
    data: [Complex; SIZE],
}

impl Block {
    pub const ZERO: Self = Self {
        data: [Complex { re: 0.0, im: 0.0 }; SIZE],
    };
}

// The framework constructs these small SRAM blocks independently. Keeping
// large buffers outside State avoids large temporary return values on the
// firmware stack. The unused imaginary lanes in table blocks are deliberate.
pub const BLOCKS: usize = LEVELS * 2 + 3;
pub type Memory = [Block; BLOCKS];

#[derive(Clone, Copy)]
pub struct Engine {
    requested: Wave,
    building: Wave,
    waves: [Wave; 2],
    active: usize,
    stage: usize,
    dirty: bool,
    initialized: bool,
    fade: usize,
    fade_length: usize,
    phase: u32,
    level: f32,
    increment: f32,
}

impl Engine {
    pub fn new(count: usize, sample_rate: u32) -> Self {
        let wave = Wave::new(count);
        Self {
            requested: wave,
            building: wave,
            waves: [wave; 2],
            active: 0,
            stage: 0,
            dirty: false,
            initialized: false,
            fade: 0,
            fade_length: (sample_rate as usize / 200).max(1),
            phase: 0,
            level: 0.0,
            increment: 0.0,
        }
    }

    pub fn request(&mut self, wave: Wave) {
        self.requested = wave;
        let latest = if self.fade > 0 {
            1 - self.active
        } else {
            self.active
        };
        self.dirty = self.requested != self.waves[latest & 1];
    }

    pub fn advance(&mut self, memory: &mut Memory) {
        if self.fade > 0 {
            return;
        }
        if self.stage == 0 {
            if !self.dirty {
                return;
            }
            self.building = self.requested;
            // Bypass the entire table/filter path at zero smoothing.
            if self.building.smoothing == 0 {
                self.publish();
                return;
            }
        }
        let (banks, workspace) = memory.split_at_mut(LEVELS * 2);
        let [spectrum, scratch, twiddles] = workspace else {
            return;
        };
        if !self.initialized {
            for (i, twiddle) in twiddles.data.iter_mut().take(SIZE / 2).enumerate() {
                let angle = core::f32::consts::TAU * i as f32 / SIZE as f32;
                *twiddle = Complex {
                    re: sine(angle + core::f32::consts::FRAC_PI_2),
                    im: -sine(angle),
                };
            }
            self.initialized = true;
        }
        if self.stage == 0 {
            for (i, value) in spectrum.data.iter_mut().enumerate() {
                *value = Complex {
                    re: self.building.sample(i as f32 / SIZE as f32),
                    im: 0.0,
                };
            }
            fft(&mut spectrum.data, &twiddles.data, false);
            self.stage = 1;
            return;
        }
        let smoothing = self.building.smoothing as f32 * 0.01;
        for (k, value) in scratch.data.iter_mut().enumerate() {
            let harmonic = k.min(SIZE - k);
            // A smooth low-pass in waveform space, independent of playback pitch.
            let ratio = harmonic as f32 / 4.0;
            let filtered_gain = 1.0 / (1.0 + ratio * ratio * ratio * ratio);
            // Blend toward the fully smoothed shape instead of lowering a
            // cutoff that quickly runs into the boundary constraints. The FFT
            // and boundary projection are linear, so each control step removes
            // the same share of detail, retaining the original maximum at 100%.
            let gain = 1.0 + smoothing * (filtered_gain - 1.0);
            *value = Complex {
                re: spectrum.data[k].re * gain,
                im: spectrum.data[k].im * gain,
            };
        }
        fft(&mut scratch.data, &twiddles.data, true);
        anchor_boundaries(&self.building, &mut scratch.data, &twiddles.data);
        let Some(table) = banks.get_mut((1 - self.active) & 1) else {
            return;
        };
        for (destination, value) in table.data.iter_mut().zip(&scratch.data) {
            destination.re = value.re;
        }
        self.publish();
    }

    fn publish(&mut self) {
        self.stage = 0;
        self.waves[(1 - self.active) & 1] = self.building;
        self.dirty = self.requested != self.building;
        self.fade = self.fade_length;
    }

    fn lookup(&self, memory: &Memory, bank: usize, phase: u32) -> f32 {
        let index = (phase >> 24) as usize;
        let fraction = (phase & 0x00ff_ffff) as f32 / 16_777_216.0;
        let Some(block) = memory.get(bank & 1) else {
            return 0.0;
        };
        let table = &block.data;
        table[index].re + (table[(index + 1) % SIZE].re - table[index].re) * fraction
    }

    fn wave_sample(&self, memory: &Memory, bank: usize, phase: u32) -> f32 {
        let wave = &self.waves[bank & 1];
        if wave.smoothing == 0 {
            wave.sample(phase as f32 / 4_294_967_296.0)
        } else {
            self.lookup(memory, bank, phase)
        }
    }

    pub fn tick(&mut self, memory: &Memory, frequency: f32, sample_rate: f32, level: f32) -> f32 {
        // Slew tuning and gain over about one millisecond; phase never resets.
        let slew = (1000.0 / sample_rate).min(1.0);
        // Avoid a dynamic f32::clamp assertion: its panic formatting pulls
        // non-PIC float-formatting code into the ARM relocatable object.
        let target = frequency.max(0.01).min(sample_rate * 0.45) / sample_rate;
        self.increment += slew * (target - self.increment);
        self.level += slew * (level - self.level);
        let old = self.wave_sample(memory, self.active, self.phase);
        let sample = if self.fade > 0 {
            let blend = 1.0 - self.fade as f32 / self.fade_length as f32;
            let new = self.wave_sample(memory, 1 - self.active, self.phase);
            self.fade -= 1;
            if self.fade == 0 {
                self.active = 1 - self.active;
            }
            old + (new - old) * blend
        } else {
            old
        };
        self.phase = self
            .phase
            .wrapping_add((self.increment * 4_294_967_296.0) as u32);
        sample * self.level * 5.0
    }

    /// Draw on the same segment-local grid as the editor. At zero smoothing,
    /// both traces then have exactly identical coordinates, including joins.
    pub fn display_segment_sample(&self, memory: &Memory, segment: usize, t: f32) -> f32 {
        let sample = |bank: usize| {
            let wave = &self.waves[bank & 1];
            if wave.smoothing == 0 {
                wave.segment_sample(segment, t)
            } else if segment + 1 == wave.count && t == 1.0 {
                self.lookup(memory, bank, 0)
            } else {
                let phase = (segment as f32 + t) / wave.count as f32;
                self.lookup(memory, bank, (phase * 4_294_967_296.0) as u32)
            }
        };
        let old = sample(self.active);
        if self.fade > 0 {
            old + (sample(1 - self.active) - old)
                * (1.0 - self.fade as f32 / self.fade_length as f32)
        } else {
            old
        }
    }

    #[cfg(test)]
    pub fn display_sample(&self, memory: &Memory, phase: u32) -> f32 {
        if self.fade > 0 {
            let blend = 1.0 - self.fade as f32 / self.fade_length as f32;
            let a = self.wave_sample(memory, self.active, phase);
            a + (self.wave_sample(memory, 1 - self.active, phase) - a) * blend
        } else {
            self.wave_sample(memory, self.active, phase)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_is_monotonic_bounded_and_moves_continuously_through_linear() {
        let mut wave = Wave::new(3);
        for curve in -100..=100 {
            wave.boundaries[1] = -100;
            wave.boundaries[2] = 100;
            wave.curves[1] = curve;
            let mut previous = -1.0;
            for i in 0..100 {
                let t = i as f32 / 100.0;
                let value = wave.sample((1.0 + t) / 3.0);
                assert!(value >= previous - 0.00001 && value <= 1.0);
                previous = value;
            }
        }
        wave.curves[1] = 0;
        assert!(wave.sample(0.5).abs() < 0.00001);
        wave.curves[1] = -1;
        assert!(wave.sample(0.5) > 0.0);
        wave.curves[1] = 1;
        assert!(wave.sample(0.5) < 0.0);
    }

    #[test]
    fn editable_seam_survives_boundary_edits_and_smoothing() {
        for count in [2, 3, 32] {
            let mut engine = Engine::new(count, 48000);
            let mut memory = [Block::ZERO; BLOCKS];
            let mut wave = Wave::new(count);
            for smoothing in [0, 50, 100] {
                wave.smoothing = smoothing;
                wave.boundaries.fill(100);
                wave.boundaries[0] = -37;
                wave.curves.fill(99);
                assert_eq!(wave.sample(0.0), -0.37);
                assert_eq!(wave.sample(1.0), -0.37);
                engine.request(wave);
                for _ in 0..50 {
                    engine.advance(&mut memory);
                    for _ in 0..128 {
                        let output = engine.tick(&memory, 440.0, 48000.0, 1.0);
                        assert!(output.is_finite() && output.abs() < 10.0);
                    }
                }
                if smoothing > 0 {
                    assert_eq!(memory[engine.active * LEVELS].data[0].re, -0.37);
                }
                assert_eq!(engine.display_segment_sample(&memory, 0, 0.0), -0.37);
                assert_eq!(
                    engine.display_segment_sample(&memory, count - 1, 1.0),
                    -0.37
                );
                assert_eq!(engine.waves[engine.active], wave);
            }
        }
    }

    #[test]
    fn smoothing_keeps_changing_the_shape_through_the_upper_control_range() {
        for n in [2, 8, 32] {
            let mut wave = Wave::new(n);
            for i in 0..n {
                wave.boundaries[i] = if i % 2 == 0 { -75 } else { 100 };
                wave.curves[i] = if i % 2 == 0 { -100 } else { 100 };
            }
            let mut previous = [0.0; SIZE];
            for smoothing in [30, 40, 50, 60, 70, 80, 90, 100] {
                let mut engine = Engine::new(n, 48000);
                let mut memory = [Block::ZERO; BLOCKS];
                wave.smoothing = smoothing;
                engine.request(wave);
                for _ in 0..2 {
                    engine.advance(&mut memory);
                }
                for _ in 0..240 {
                    engine.tick(&memory, 220.0, 48000.0, 1.0);
                }
                let mut difference = 0.0;
                for (i, old) in previous.iter_mut().enumerate() {
                    let sample = engine.display_sample(&memory, (i as u32) << 24);
                    difference += (sample - *old) * (sample - *old);
                    *old = sample;
                }
                if smoothing > 30 {
                    // Each upper-range step must still change the actual output,
                    // not just the parameter value or an intermediate filter.
                    assert!(
                        difference / SIZE as f32 > 0.000001,
                        "smoothing {smoothing}, segments {n}: mean-square change {}",
                        difference / SIZE as f32
                    );
                }
                assert_eq!(engine.display_segment_sample(&memory, 0, 0.0), -0.75);
                assert_eq!(engine.display_segment_sample(&memory, n - 1, 1.0), -0.75);
            }
        }
    }

    #[test]
    fn smoothing_fft_round_trip() {
        let mut engine = Engine::new(4, 48000);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut wave = Wave::new(4);
        wave.smoothing = 50;
        engine.request(wave);
        for _ in 0..=LEVELS {
            engine.advance(&mut memory);
        }
        for _ in 0..240 {
            engine.tick(&memory, 220.0, 48000.0, 1.0);
        }
        let mut signal = [Complex::default(); SIZE];
        for (i, value) in signal.iter_mut().enumerate() {
            value.re = sine(core::f32::consts::TAU * i as f32 / SIZE as f32);
        }
        let original = signal;
        fft(&mut signal, &memory[TWIDDLES].data, false);
        fft(&mut signal, &memory[TWIDDLES].data, true);
        for (a, b) in signal.iter().zip(original) {
            assert!((a.re - b.re).abs() < 0.00001);
        }
    }

    #[test]
    fn smoothing_reduces_high_harmonics_and_rebuild_crossfade_starts_at_old_wave() {
        let mut engine = Engine::new(8, 48000);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut wave = Wave::new(8);
        wave.boundaries[2] = 100;
        wave.boundaries[3] = -100;
        wave.boundaries[4] = 100;
        wave.curves[2] = 100;
        engine.request(wave);
        for _ in 0..=LEVELS {
            engine.advance(&mut memory);
        }
        for _ in 0..240 {
            engine.tick(&memory, 220.0, 48000.0, 1.0);
        }
        let mut spectrum = [Complex::default(); SIZE];
        for (i, sample) in spectrum.iter_mut().enumerate() {
            sample.re = wave.sample(i as f32 / SIZE as f32);
        }
        // Initial zero-smoothing playback intentionally never initializes FFT
        // tables, so obtain twiddles from a separate smoothed build.
        let mut smoothed = wave;
        smoothed.smoothing = 100;
        engine.request(smoothed);
        for _ in 0..=LEVELS {
            engine.advance(&mut memory);
        }
        let old = engine.display_sample(&memory, engine.phase);
        fft(&mut spectrum, &memory[TWIDDLES].data, false);
        let before: f32 = spectrum[16..64]
            .iter()
            .map(|s| s.re * s.re + s.im * s.im)
            .sum();
        assert_eq!(engine.fade, engine.fade_length);
        assert_eq!(engine.display_sample(&memory, engine.phase), old);
        for _ in 0..240 {
            engine.tick(&memory, 220.0, 48000.0, 1.0);
        }
        spectrum = memory[engine.active * LEVELS].data;
        fft(&mut spectrum, &memory[TWIDDLES].data, false);
        let after: f32 = spectrum[16..64]
            .iter()
            .map(|s| s.re * s.re + s.im * s.im)
            .sum();
        assert!(before > 0.1 && after < before * 0.01);
    }

    #[test]
    fn curve_and_smoothing_preserve_continuous_boundary_levels() {
        for n in 2..=MAX_SEGMENTS {
            for curve in [-100, 0, 100] {
                let mut wave = Wave::new(n);
                wave.boundaries[0] = 63;
                wave.curves.fill(curve);
                for smoothing in [0, 50, 100] {
                    wave.smoothing = smoothing;
                    let mut engine = Engine::new(n, 48000);
                    let mut memory = [Block::ZERO; BLOCKS];
                    engine.request(wave);
                    for _ in 0..=LEVELS {
                        engine.advance(&mut memory);
                    }
                    for _ in 0..240 {
                        engine.tick(&memory, 20.0, 48000.0, 1.0);
                    }
                    for i in 0..n {
                        let phase = (i as f64 / n as f64 * 4_294_967_296.0) as u32;
                        let expected = wave.segment_sample(i, 0.0);
                        assert!((engine.display_sample(&memory, phase) - expected).abs() < 0.001);
                    }
                }
            }
        }
    }

    #[test]
    fn neighboring_segments_share_boundaries_for_every_curve() {
        let mut wave = Wave::new(8);
        wave.boundaries[2] = 75;
        wave.boundaries[3] = -25;
        for curve in -100..=100 {
            wave.curves[2] = curve;
            wave.curves[1] = -curve;
            wave.curves[3] = -curve;
            assert_eq!(wave.segment_sample(2, 0.0), 0.75);
            assert_eq!(wave.segment_sample(2, 1.0), -0.25);
            assert_eq!(wave.segment_sample(1, 1.0), wave.segment_sample(2, 0.0));
            assert_eq!(wave.segment_sample(2, 1.0), wave.segment_sample(3, 0.0));
        }
        for curve in [-100, 0, 100] {
            wave.curves[2] = curve;
            let mut engine = Engine::new(8, 48000);
            let mut memory = [Block::ZERO; BLOCKS];
            engine.request(wave);
            for _ in 0..=LEVELS {
                engine.advance(&mut memory);
            }
            for _ in 0..240 {
                engine.tick(&memory, 20.0, 48000.0, 1.0);
            }
            let left_target = 0.75;
            let right_target = -0.25;
            assert!((engine.display_sample(&memory, 1 << 30) - left_target).abs() < 0.0001);
            assert!((engine.display_sample(&memory, 3 << 29) - right_target).abs() < 0.0001);
        }
    }

    #[test]
    fn smoothing_is_pitch_independent_and_returns_to_exact_source_at_zero() {
        for n in [3, 7, 16, 31, 32] {
            let mut engine = Engine::new(n, 48000);
            let mut memory = [Block::ZERO; BLOCKS];
            let mut wave = Wave::new(n);
            wave.smoothing = 80;
            for (i, boundary) in wave.boundaries.iter_mut().take(n).enumerate() {
                *boundary = if i % 2 == 0 { 100 } else { -75 };
            }
            wave.curves.fill(100);
            engine.request(wave);
            for _ in 0..=LEVELS {
                engine.advance(&mut memory);
            }
            for _ in 0..240 {
                engine.tick(&memory, 20.0, 48000.0, 1.0);
            }
            let mut preview = [0.0; SIZE];
            let mut difference = 0.0_f32;
            for (i, sample) in preview.iter_mut().enumerate() {
                let phase = (i as u32) << 24;
                *sample = engine.display_sample(&memory, phase);
                difference = difference.max((*sample - wave.sample(i as f32 / SIZE as f32)).abs());
            }
            assert!(difference > 0.01);
            for _ in 0..1000 {
                engine.tick(&memory, 10000.0, 48000.0, 1.0);
            }
            for (i, &sample) in preview.iter().enumerate() {
                assert_eq!(engine.display_sample(&memory, (i as u32) << 24), sample);
            }
            wave.smoothing = 0;
            engine.request(wave);
            engine.advance(&mut memory);
            for _ in 0..240 {
                engine.tick(&memory, 10000.0, 48000.0, 1.0);
            }
            for i in 0..1024 {
                let phase = (i as u32).wrapping_mul(4_194_301);
                assert_eq!(
                    engine.display_sample(&memory, phase),
                    wave.sample(phase as f32 / 4_294_967_296.0)
                );
            }
        }
    }

    #[test]
    fn zero_smoothing_audio_and_preview_evaluate_the_exact_curve_at_any_pitch() {
        for n in [2, 7, 32] {
            for curve in [-100, 0, 100] {
                let mut wave = Wave::new(n);
                for i in 1..n {
                    wave.boundaries[i] = if i % 2 == 0 { 100 } else { -75 };
                }
                wave.curves.fill(curve);
                let mut engine = Engine::new(n, 48000);
                let mut memory = [Block::ZERO; BLOCKS];
                engine.request(wave);
                engine.advance(&mut memory);
                for _ in 0..240 {
                    engine.tick(&memory, 220.0, 48000.0, 1.0);
                }
                assert!(
                    !engine.initialized,
                    "zero smoothing must bypass FFT initialization"
                );
                // Poison the unused table banks to prove neither the audio nor
                // preview reads a sampled approximation of the source curve.
                for bank in memory.iter_mut().take(2) {
                    for sample in &mut bank.data {
                        sample.re = f32::NAN;
                    }
                }
                for frequency in [20.0, 440.0, 10000.0] {
                    engine.increment = frequency / 48000.0;
                    engine.level = 1.0;
                    for _ in 0..1024 {
                        let phase = engine.phase;
                        let expected = wave.sample(phase as f32 / 4_294_967_296.0);
                        assert_eq!(engine.display_sample(&memory, phase), expected);
                        assert_eq!(
                            engine.tick(&memory, frequency, 48000.0, 1.0),
                            expected * 5.0
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn continuous_edits_publish_without_starvation_and_keep_phase() {
        let mut engine = Engine::new(32, 48000);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut wave = Wave::new(32);
        for block in 0..100 {
            wave.boundaries[4] = block;
            engine.request(wave);
            engine.advance(&mut memory);
            for _ in 0..64 {
                assert!(engine.tick(&memory, 10000.0, 48000.0, 1.0).is_finite());
            }
        }
        assert!(engine.waves[engine.active].boundaries[4] > 70);
        assert_ne!(engine.phase, 0);
    }
}
