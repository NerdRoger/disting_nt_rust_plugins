//! Segmented oscillator with pitch-selected band-limited tables. Creative
//! smoothing precedes the final harmonic cutoff. Edits crossfade over five
//! milliseconds without resetting phase.
#![forbid(unsafe_code)]

pub const SIZE: usize = 256;
pub const DEFAULT_TABLES: usize = 6;
pub const MAX_TABLES: usize = 20;
#[cfg(test)]
const LEVELS: usize = DEFAULT_TABLES;
#[cfg(test)]
const TABLES_PER_BANK: usize = LEVELS + 1;
// Pack two real-valued tables into each block's real/imaginary lanes.
#[cfg(test)]
const BANK_BLOCKS: usize = TABLES_PER_BANK;
const TABLES_PER_STEP: usize = 4;
pub const MAX_SEGMENTS: usize = 32;
#[cfg(test)]
const TWIDDLES: usize = BANK_BLOCKS + 2;

#[cfg(test)]
const BUILD_STEPS: usize = LEVELS.div_ceil(TABLES_PER_STEP) + 2;

#[derive(Clone, Copy)]
struct TableLayout {
    count: usize,
    cutoffs: [u8; MAX_TABLES],
}

impl TableLayout {
    fn new(count: usize) -> Self {
        let count = count.clamp(1, MAX_TABLES);
        // Refinements are nested: increasing the specification adds a table
        // without discarding existing limits. Six matches the listening prototype.
        let priority = [
            127, 1, 8, 32, 4, 2, 16, 64, 6, 12, 3, 48, 24, 96, 5, 10, 20, 40, 80, 112,
        ];
        let mut cutoffs = [0u8; MAX_TABLES];
        for (to, from) in cutoffs.iter_mut().zip(priority).take(count) {
            *to = from;
        }
        cutoffs.sort_unstable_by(|a, b| b.cmp(a));
        Self { count, cutoffs }
    }

    fn cutoff(&self, level: usize) -> usize {
        self.cutoffs.get(level).copied().unwrap_or(1) as usize
    }

    fn playback_levels(&self, increment: f32) -> (usize, f32) {
        if self.count == 1 {
            return (0, 0.0);
        }
        // Binary search keeps pitch selection bounded to five comparisons.
        let (mut first, mut last) = (0, self.count - 1);
        while first < last {
            let middle = (first + last) / 2;
            if increment * self.cutoff(middle) as f32 >= 0.49 {
                first = middle + 1;
            } else {
                last = middle;
            }
        }
        let fade = ((increment * self.cutoff(first) as f32 - 0.47) / 0.02).clamp(0.0, 1.0);
        (first, fade * fade * (3.0 - 2.0 * fade))
    }
}

#[cfg(test)]
fn cutoff(level: usize) -> usize {
    TableLayout::new(DEFAULT_TABLES).cutoff(level)
}
#[cfg(test)]
fn playback_levels(increment: f32) -> (usize, f32) {
    TableLayout::new(DEFAULT_TABLES).playback_levels(increment)
}

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
/// independent of oscillator pitch. The final anti-alias filter follows this.
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
// firmware stack. Each table uses one real or imaginary lane in a packed block.
pub const fn blocks(tables: usize) -> usize {
    tables + 4
}
#[cfg(test)]
const BLOCKS: usize = blocks(DEFAULT_TABLES);
pub type Memory = [Block];

fn store_table(banks: &mut [Block], index: usize, source: &[Complex; SIZE]) -> Option<()> {
    let destination = banks.get_mut(index / 2)?;
    for (to, from) in destination.data.iter_mut().zip(source) {
        if index & 1 == 0 {
            to.re = from.re;
        } else {
            to.im = from.re;
        }
    }
    Some(())
}

#[derive(Clone, Copy)]
pub struct Engine {
    layout: TableLayout,
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
    ready: [bool; 2],
    playback_level: usize,
    playback_blend: f32,
}

impl Engine {
    #[cfg(test)]
    pub fn new(count: usize, sample_rate: u32) -> Self {
        Self::with_tables(count, sample_rate, DEFAULT_TABLES)
    }

    pub fn with_tables(count: usize, sample_rate: u32, tables: usize) -> Self {
        let wave = Wave::new(count);
        Self {
            layout: TableLayout::new(tables),
            requested: wave,
            building: wave,
            waves: [wave; 2],
            active: 0,
            stage: 0,
            dirty: true,
            initialized: false,
            fade: 0,
            fade_length: (sample_rate as usize / 200).max(1),
            phase: 0,
            level: 0.0,
            increment: 0.0,
            ready: [false; 2],
            playback_level: 0,
            playback_blend: 0.0,
        }
    }

    pub fn request(&mut self, wave: Wave) {
        self.requested = wave;
        let latest = if self.fade > 0 {
            1 - self.active
        } else {
            self.active
        };
        self.dirty = !self.ready[latest & 1] || self.requested != self.waves[latest & 1];
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
        }
        let tables_per_bank = self.layout.count + 1;
        if memory.len() != blocks(self.layout.count) {
            return;
        }
        let (banks, workspace) = memory.split_at_mut(tables_per_bank);
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
        if self.stage == 1 {
            if self.building.smoothing > 0 {
                let smoothing = self.building.smoothing as f32 * 0.01;
                for (k, value) in scratch.data.iter_mut().enumerate() {
                    let harmonic = k.min(SIZE - k);
                    let ratio = harmonic as f32 / 4.0;
                    let filtered_gain = 1.0 / (1.0 + ratio * ratio * ratio * ratio);
                    let gain = 1.0 + smoothing * (filtered_gain - 1.0);
                    *value = Complex {
                        re: spectrum.data[k].re * gain,
                        im: spectrum.data[k].im * gain,
                    };
                }
                fft(&mut scratch.data, &twiddles.data, true);
                anchor_boundaries(&self.building, &mut scratch.data, &twiddles.data);
                // Retain the creative shape for the editor before any playback
                // harmonic cutoff. Publish it together with the audio tables.
                if store_table(
                    banks,
                    (1 - self.active) * tables_per_bank + self.layout.count,
                    &scratch.data,
                )
                .is_none()
                {
                    return;
                }
                // Re-enter the frequency domain after anchoring. Never anchor
                // the band-limited tables: doing so would restore unsafe harmonics.
                fft(&mut scratch.data, &twiddles.data, false);
                spectrum.data.copy_from_slice(&scratch.data);
            }
            self.stage = 2;
            return;
        }
        let first_level = (self.stage - 2) * TABLES_PER_STEP;
        for level in first_level..(first_level + TABLES_PER_STEP).min(self.layout.count) {
            let cutoff = self.layout.cutoff(level);
            for (k, value) in scratch.data.iter_mut().enumerate() {
                let harmonic = k.min(SIZE - k);
                *value = if self.layout.count == 1 || (harmonic <= cutoff && harmonic < SIZE / 2) {
                    spectrum.data[k]
                } else {
                    Complex::default()
                };
            }
            fft(&mut scratch.data, &twiddles.data, true);
            if store_table(
                banks,
                (1 - self.active) * tables_per_bank + level,
                &scratch.data,
            )
            .is_none()
            {
                return;
            }
        }
        self.stage += 1;
        if first_level + TABLES_PER_STEP >= self.layout.count {
            self.publish();
        }
    }

    fn publish(&mut self) {
        self.stage = 0;
        self.waves[(1 - self.active) & 1] = self.building;
        self.ready[(1 - self.active) & 1] = true;
        self.dirty = self.requested != self.building;
        self.fade = self.fade_length;
    }

    fn lookup(&self, memory: &Memory, table_index: usize, phase: u32) -> f32 {
        let index = (phase >> 24) as usize;
        let fraction = (phase & 0x00ff_ffff) as f32 / 16_777_216.0;
        let Some(block) = memory.get(table_index / 2) else {
            return 0.0;
        };
        let table = &block.data;
        let (left, right) = if table_index & 1 == 0 {
            (table[index].re, table[(index + 1) & (SIZE - 1)].re)
        } else {
            (table[index].im, table[(index + 1) & (SIZE - 1)].im)
        };
        left + (right - left) * fraction
    }

    fn wave_sample(&self, memory: &Memory, bank: usize, phase: u32) -> f32 {
        if !self.ready[bank & 1] {
            return 0.0;
        }
        let index = (bank & 1) * (self.layout.count + 1) + self.playback_level;
        let rich = self.lookup(memory, index, phase);
        if self.playback_blend == 0.0 || self.playback_level + 1 == self.layout.count {
            return rich;
        }
        rich + (self.lookup(memory, index + 1, phase) - rich) * self.playback_blend
    }
    pub fn tick(&mut self, memory: &Memory, frequency: f32, sample_rate: f32, level: f32) -> f32 {
        // Slew tuning and gain over about one millisecond; phase never resets.
        let slew = (1000.0 / sample_rate).min(1.0);
        // Avoid a dynamic f32::clamp assertion: its panic formatting pulls
        // non-PIC float-formatting code into the ARM relocatable object.
        let target = frequency.max(0.01).min(sample_rate * 0.45) / sample_rate;
        self.increment += slew * (target - self.increment);
        (self.playback_level, self.playback_blend) = self.layout.playback_levels(self.increment);
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

    /// Draw the defined shape plus creative smoothing, independent of pitch.
    /// The short edit crossfade remains shared with audio to keep edits stable.
    pub fn display_segment_sample(&self, memory: &Memory, segment: usize, t: f32) -> f32 {
        let sample = |bank: usize| {
            let wave = &self.waves[bank & 1];
            if wave.smoothing == 0 {
                wave.segment_sample(segment, t)
            } else if segment + 1 == wave.count && t == 1.0 {
                self.lookup(
                    memory,
                    (bank & 1) * (self.layout.count + 1) + self.layout.count,
                    0,
                )
            } else {
                let phase = (segment as f32 + t) / wave.count as f32;
                self.lookup(
                    memory,
                    (bank & 1) * (self.layout.count + 1) + self.layout.count,
                    (phase * 4_294_967_296.0) as u32,
                )
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

    /// Audio-path shape before output Level, including pitch filtering and edit fades.
    pub fn audio_segment_sample(&self, memory: &Memory, segment: usize, t: f32) -> f32 {
        let n = self.waves[self.active & 1].count.max(2);
        let phase = if segment + 1 == n && t == 1.0 {
            0
        } else {
            (((segment as f32 + t) / n as f32) * 4_294_967_296.0) as u32
        };
        self.display_sample(memory, phase)
    }

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
#[path = "dsp_tests.rs"]
mod tests;
