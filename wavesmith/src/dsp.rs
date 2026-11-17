//! Segmented oscillator with pitch-selected band-limited tables. Creative
//! smoothing precedes the final harmonic cutoff. Edits crossfade over five
//! milliseconds without resetting phase.
#![forbid(unsafe_code)]

pub const SIZE: usize = 256;
pub const WARP_POSITION_MAX: i16 = 255;
pub const WARP_POSITION_DEFAULT: i16 = 128;
pub const MAX_FOLDS: i16 = 2;
pub const DEFAULT_FOLD_LIMIT: i16 = 2;
pub const DEFAULT_TABLES: usize = 6;
pub const MAX_TABLES: usize = 20;
// FFT uses 0..128; packed smoothing gains use 128..193. Cache geometry in
// the remaining twiddle-buffer space: 33 positions and 32 reciprocal widths.
const POSITION_CACHE: usize = SIZE / 2 + (SIZE / 2 + 1).div_ceil(2);
pub const MAX_SEGMENTS: usize = 32;

#[derive(Clone, Copy)]
pub enum BoundaryShape {
    Sine,
    Triangle,
    Square,
    Sawtooth,
}

pub fn preset_boundary(shape: BoundaryShape, boundary: usize, count: usize) -> i16 {
    let count = count.clamp(2, MAX_SEGMENTS);
    let phase = (boundary % count) as f32 / count as f32;
    // Start sine/triangle at their negative peak so two-segment instances
    // remain bipolar rather than sampling only the sine's zero crossings.
    let value = match shape {
        BoundaryShape::Sine => -sine(core::f32::consts::TAU * phase + core::f32::consts::FRAC_PI_2),
        BoundaryShape::Triangle => {
            if phase < 0.5 {
                -1.0 + 4.0 * phase
            } else {
                3.0 - 4.0 * phase
            }
        }
        BoundaryShape::Square => {
            if phase < 0.5 {
                -1.0
            } else {
                1.0
            }
        }
        // Reach the positive rail at the last interior boundary, then return
        // to the shared negative seam over the final segment.
        BoundaryShape::Sawtooth => -1.0 + 2.0 * (boundary % count) as f32 / (count - 1) as f32,
    };
    let scaled = value * 100.0;
    ((if scaled < 0.0 {
        scaled - 0.5
    } else {
        scaled + 0.5
    }) as i16)
        .clamp(-100, 100)
}
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug, PartialEq))]
struct TableLayout {
    count: usize,
    cutoffs: [u8; MAX_TABLES],
}

impl TableLayout {
    #[inline(always)]
    fn new(count: usize) -> Self {
        let count = count.clamp(1, MAX_TABLES);
        // Nested refinements retain existing harmonic limits. Production uses six;
        // other counts remain available to the DSP regression tests.
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
    pub warp_position: i16,
    pub warp_amount: i16,
    pub fold_limit: i16,
    pub boundaries: [i16; MAX_SEGMENTS + 1],
    pub curves: [i16; MAX_SEGMENTS],
}

impl Wave {
    /// Initialize storage without calculating a preset that callers will replace.
    pub fn blank(count: usize) -> Self {
        Self {
            count: count.clamp(2, MAX_SEGMENTS),
            smoothing: 0,
            warp_position: WARP_POSITION_DEFAULT,
            warp_amount: 0,
            fold_limit: DEFAULT_FOLD_LIMIT,
            boundaries: [0; MAX_SEGMENTS + 1],
            curves: [0; MAX_SEGMENTS],
        }
    }

    #[inline(always)]
    pub fn new(count: usize) -> Self {
        let mut wave = Self::blank(count);
        for i in 0..wave.count {
            wave.boundaries[i] = default_level(i, wave.count);
        }
        wave
    }

    pub fn sample(&self, phase: f32) -> f32 {
        if phase <= 0.0 || phase >= 1.0 {
            return self.boundary_level(0);
        }
        let (index, t) = self.segment_at(phase);
        self.segment_sample(index, t)
    }

    // Evaluate equal-width coordinates without changing the stored build shape.
    fn sample_unwarped(&self, phase: f32) -> f32 {
        if phase <= 0.0 || phase >= 1.0 {
            return self.boundary_level(0);
        }
        let position = phase * self.count as f32;
        let index = (position as usize).min(self.count - 1);
        self.segment_sample(index, position - index as f32)
    }

    pub fn boundary_position(&self, boundary: usize) -> f32 {
        let x = boundary.min(self.count) as f32 / self.count.max(2) as f32;
        if boundary == 0 || boundary >= self.count || self.warp_amount == 0 {
            return x;
        }
        let center =
            self.warp_position.clamp(0, WARP_POSITION_MAX) as f32 / WARP_POSITION_MAX as f32;
        let amount = self.warp_amount.clamp(-100, 100) as f32 * 0.0095;
        let extent = if x < center { center } else { 1.0 - center };
        if extent <= 0.0 {
            return x;
        }
        let distance = (x - center).abs() / extent;
        // Monotone rational maps keep both edges and the warp position fixed. The
        // 0.95 cap prevents zero-width segments even at full attraction/repulsion.
        let moved = if amount > 0.0 {
            distance / (1.0 - amount + amount * distance)
        } else {
            distance * (1.0 + amount) / (1.0 + amount * distance)
        };
        if x < center {
            center - extent * moved
        } else {
            center + extent * moved
        }
    }

    #[cfg(test)]
    pub fn segment_phase(&self, segment: usize, t: f32) -> f32 {
        let start = self.boundary_position(segment);
        start + (self.boundary_position(segment + 1) - start) * t
    }

    fn segment_at(&self, phase: f32) -> (usize, f32) {
        if self.warp_amount == 0 {
            let position = phase * self.count as f32;
            let index = (position as usize).min(self.count - 1);
            return (index, position - index as f32);
        }
        let (mut first, mut last) = (0, self.count - 1);
        while first < last {
            let middle = (first + last).div_ceil(2);
            if self.boundary_position(middle) <= phase {
                first = middle;
            } else {
                last = middle - 1;
            }
        }
        let start = self.boundary_position(first);
        let width = self.boundary_position(first + 1) - start;
        (first, ((phase - start) / width).clamp(0.0, 1.0))
    }

    fn unwarped_phase(&self, phase: f32) -> f32 {
        if self.warp_amount == 0 {
            return phase;
        }
        let (index, t) = self.segment_at(phase);
        (index as f32 + t) / self.count as f32
    }

    pub fn boundary_level(&self, boundary: usize) -> f32 {
        let boundary = if boundary >= self.count { 0 } else { boundary };
        self.boundaries[boundary.min(MAX_SEGMENTS)].clamp(-100, 100) as f32 * 0.01
    }

    /// Both sides of every join read the same boundary value. The outer
    /// boundaries share the editable seam value stored at boundary zero.
    pub fn segment_sample(&self, index: usize, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
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
        let bend = self.curves[index.min(MAX_SEGMENTS - 1)].clamp(-100, 100) as f32 * 0.01;
        let linear = start + (end - start) * t;
        if bend == 0.0 {
            return linear;
        }
        let folds = self.fold_limit.clamp(0, MAX_FOLDS) as usize;
        if folds == 0 {
            // A bounded quadratic Bezier control point keeps curvature active
            // without overshooting either global rail or introducing clipping.
            let midpoint = (start + end) * 0.5;
            let headroom = if bend > 0.0 {
                1.0 - midpoint
            } else {
                1.0 + midpoint
            };
            return linear + 2.0 * bend * headroom * t * (1.0 - t);
        }
        // Limit the unfolded excursion to +/- (1 + 2*folds), so every sample
        // returns to the rails within the allowed reflection count. At 2 this
        // is the original +/-4 midpoint bow and identical curve arithmetic.
        let value = linear + (8.0 * folds as f32) * bend * t * (1.0 - t);
        fold(value, folds)
    }
}

// Excursion is bounded before reflection; never truncate an out-of-range
// waveform or clip its peaks. Stop early when no further reflection is needed.
fn fold(mut value: f32, limit: usize) -> f32 {
    for _ in 0..limit {
        if value > 1.0 {
            value = 2.0 - value;
        } else if value < -1.0 {
            value = -2.0 - value;
        } else {
            break;
        }
    }
    value
}

// The FFT needs only the first half-cycle. Recover the remaining phases by
// symmetry, so harmonic synthesis can reuse those values without a new buffer.
fn cached_geometry(twiddles: &[Complex; SIZE], index: usize) -> f32 {
    let value = twiddles[POSITION_CACHE + index.min(MAX_SEGMENTS * 2) / 2];
    if index & 1 == 0 {
        value.re
    } else {
        value.im
    }
}

fn set_cached_geometry(twiddles: &mut [Complex; SIZE], index: usize, value: f32) {
    let slot = &mut twiddles[POSITION_CACHE + index.min(MAX_SEGMENTS * 2) / 2];
    if index & 1 == 0 {
        slot.re = value;
    } else {
        slot.im = value;
    }
}

#[inline(always)]
fn prepare_geometry(wave: &Wave, twiddles: &mut [Complex; SIZE]) {
    for i in 0..=wave.count {
        set_cached_geometry(twiddles, i, wave.boundary_position(i));
    }
    for i in 0..wave.count {
        let width = cached_geometry(twiddles, i + 1) - cached_geometry(twiddles, i);
        set_cached_geometry(twiddles, MAX_SEGMENTS + 1 + i, 1.0 / width);
    }
}

// Build samples advance monotonically in phase. Retain the cursor between
// chunks, visiting each boundary at most once instead of searching per sample.
fn cached_segment_at(
    wave: &Wave,
    twiddles: &[Complex; SIZE],
    phase: f32,
    cursor: &mut usize,
) -> (usize, f32) {
    while *cursor + 1 < wave.count && cached_geometry(twiddles, *cursor + 1) <= phase {
        *cursor += 1;
    }
    let start = cached_geometry(twiddles, *cursor);
    let inverse_width = cached_geometry(twiddles, MAX_SEGMENTS + 1 + *cursor);
    (*cursor, ((phase - start) * inverse_width).clamp(0.0, 1.0))
}

pub fn default_level(node: usize, count: usize) -> i16 {
    preset_boundary(BoundaryShape::Sine, node, count)
}

#[derive(Clone, Copy, Default)]
pub struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    pub fn initialize(destination: &mut distingnt_api::api::plugin::RegionInitializer<'_, Self>) {
        distingnt_api::initialize_region!(destination, Self { re: 0.0, im: 0.0 });
    }
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
    #[cfg(test)]
    {
        tests::count_fft(inverse);
        crate::tests::count_custom_fft();
    }
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

// Each callback performs four of the eight radix-2 stages. Reordering belongs
// to the first half; inverse normalization belongs to the second. No other
// build work may touch this buffer between these two calls.
#[derive(Clone, Copy)]
pub struct Block {
    data: [Complex; SIZE],
}

impl Block {
    pub fn initialize(destination: &mut distingnt_api::api::plugin::RegionInitializer<'_, Self>) {
        distingnt_api::initialize_region!(
            destination,
            Block {
                data: [Complex { re: 0.0, im: 0.0 }; SIZE],
            }
        );
    }
    /// Interpolate one real-valued table stored in either lane of this block.
    /// Inline into the render loops so table/lane decisions stay hoisted there.
    #[inline(always)]
    fn interpolate(&self, imaginary: bool, index: usize, next: usize, fraction: f32) -> f32 {
        let (left, right) = if imaginary {
            (self.data[index].im, self.data[next].im)
        } else {
            (self.data[index].re, self.data[next].re)
        };
        left + (right - left) * fraction
    }

    #[cfg(test)]
    pub const ZERO: Self = Self {
        data: [Complex { re: 0.0, im: 0.0 }; SIZE],
    };
}

// Packed real/imaginary lanes hold the two production playback banks.
pub type Memory = [Block];

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub struct CvFactor {
    bus: i16,
    voltage: f32,
    factor: f32,
}

/// Cursor over at most one distinct input per voice. Entries live in the
/// specification-sized voice records and retain exact inputs between callbacks.
#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub struct CvPitchCache {
    used: usize,
}

impl CvFactor {
    pub const fn new() -> Self {
        Self {
            bus: -1,
            voltage: f32::INFINITY,
            factor: 1.0,
        }
    }
}

impl CvPitchCache {
    #[inline(always)]
    pub fn initialize(destination: &mut distingnt_api::api::plugin::RegionInitializer<'_, Self>) {
        distingnt_api::initialize_region!(destination, CvPitchCache { used: 0 });
    }
    #[cfg(test)]
    pub const fn new() -> Self {
        Self { used: 0 }
    }
    pub fn begin_block(&mut self) {
        self.used = 0;
    }
    #[inline(always)]
    pub fn factor<'a>(
        &mut self,
        bus: i16,
        voltage: f32,
        entries: impl Iterator<Item = &'a mut CvFactor>,
    ) -> f32 {
        if voltage == 0.0 {
            return 1.0;
        }
        for (index, entry) in entries.enumerate() {
            if index < self.used && entry.bus != bus {
                continue;
            }
            if index == self.used {
                self.used += 1;
            }
            if entry.bus != bus || entry.voltage != voltage {
                entry.bus = bus;
                entry.voltage = voltage;
                entry.factor = distingnt_api::api::math::exp2(voltage);
                #[cfg(test)]
                tests::count_frequency_conversion();
            }
            return entry.factor;
        }
        distingnt_api::api::math::exp2(voltage)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oscillator {
    phase: u32,
    level: f32,
    playback_level: usize,
    playback_blend: f32,
    selection_low: u32,
    selection_high: u32,
    pitch_pair_slew: i32,
    pair_end: u32,
    pair_pending: bool,
    #[cfg(test)]
    selection_step: u32,
    cached_octave: f32,
    cached_frequency: f32,
    cached_rate: f32,
    prepared_input: f32,
    cv_phase_scale: f32,
    cv_minimum_step: f32,
    target_step: u32,
    target_level: f32,
    slew: f32,
    phase_step: u32,
    pitch_moving: bool,
    level_moving: bool,
}

// 1 mV at 1 V/oct is 0.001 octaves (1.2 cents). Compare frequency ratios
// instead of converting the slewed oscillator pitch back to volts/logarithms.
const SELECTION_RATIO: f32 = 1.000_693_4; // 2^0.001
const SELECTION_RATIO_INVERSE: f32 = 0.999_307_1; // 2^-0.001

const PHASE_SCALE: f32 = 4_294_967_296.0;
const PHASE_SCALE_INVERSE: f32 = 1.0 / PHASE_SCALE;
// Q30 keeps the coefficient signed-positive even for an instantaneous slew.
const PITCH_SLEW_SCALE: f32 = 1_073_741_824.0;
const SELECTION_HIGH_Q31: u64 = (SELECTION_RATIO * 2_147_483_648.0) as u64;
const SELECTION_LOW_Q31: u64 = (SELECTION_RATIO_INVERSE * 2_147_483_648.0) as u64;

impl Oscillator {
    pub const fn new() -> Self {
        Self {
            phase: 0,
            level: 0.0,
            playback_level: 0,
            playback_blend: 0.0,
            selection_low: 0,
            selection_high: 0,
            pitch_pair_slew: 0,
            pair_end: 0,
            pair_pending: false,
            #[cfg(test)]
            selection_step: 0,
            cached_octave: f32::INFINITY,
            cached_frequency: 0.0,
            cached_rate: 0.0,
            prepared_input: 0.0,
            cv_phase_scale: 0.0,
            cv_minimum_step: 0.0,
            target_step: 0,
            target_level: 0.0,
            slew: 0.0,
            phase_step: 0,
            pitch_moving: false,
            level_moving: false,
        }
    }
    #[cfg(test)]
    pub fn target_frequency_for_test(&self) -> f32 {
        self.target_step as f32 * PHASE_SCALE_INVERSE * self.cached_rate
    }

    /// Cache the exact exponent: MIDI note/bend/fine, or CV-mode root/transpose/fine.
    pub fn frequency(&mut self, octave: f32) -> f32 {
        if octave != self.cached_octave {
            self.cached_octave = octave;
            self.cached_frequency = 440.0 * distingnt_api::api::math::exp2(octave);
            #[cfg(test)]
            tests::count_frequency_conversion();
        }
        self.cached_frequency
    }

    pub fn prepare_cv(&mut self, tuning: f32, factor: f32, sample_rate: f32, level: f32) {
        let tuning_changed = tuning != self.cached_octave;
        let rate_changed = sample_rate != self.cached_rate;
        let root_frequency = self.frequency(tuning);
        let rebuild_scale = tuning_changed || rate_changed || self.cv_phase_scale == 0.0;
        if rebuild_scale {
            self.cv_phase_scale = root_frequency / sample_rate * PHASE_SCALE;
            self.cv_minimum_step = 0.01 / sample_rate * PHASE_SCALE;
            if rate_changed {
                self.update_sample_rate(sample_rate);
            }
        }
        if rebuild_scale || factor != self.prepared_input {
            self.prepared_input = factor;
            self.pair_pending = false;
            self.target_step = (self.cv_phase_scale * factor)
                .max(self.cv_minimum_step)
                .min(0.45 * PHASE_SCALE) as u32;
            self.update_pitch_moving();
        }
        self.set_target_level(level);
    }

    pub fn prepare(&mut self, frequency: f32, sample_rate: f32, level: f32) {
        self.cv_phase_scale = 0.0;
        if frequency != self.prepared_input || sample_rate != self.cached_rate {
            self.prepared_input = frequency;
            self.pair_pending = false;
            if sample_rate != self.cached_rate {
                self.update_sample_rate(sample_rate);
            }
            let increment = frequency.max(0.01).min(sample_rate * 0.45) / sample_rate;
            self.target_step = (increment * PHASE_SCALE) as u32;
            self.update_pitch_moving();
        }
        self.set_target_level(level);
    }

    // Both input modes share the same slew coefficients and transition flags.
    fn update_sample_rate(&mut self, sample_rate: f32) {
        self.cached_rate = sample_rate;
        self.slew = (1000.0 / sample_rate).min(1.0);
        let pitch_slew = ((self.slew * PITCH_SLEW_SCALE) as i32).max(1);
        // alpha_pair = alpha + alpha*(1-alpha), rounded in Q30.
        self.pitch_pair_slew = pitch_slew
            + ((pitch_slew as i64 * ((1i64 << 30) - pitch_slew as i64) + (1i64 << 29)) >> 30)
                as i32;
    }

    fn update_pitch_moving(&mut self) {
        self.pitch_moving = self.phase_step != self.target_step
            || self.selection_high == 0
            || self.phase_step >= self.selection_high
            || self.phase_step <= self.selection_low;
    }

    fn set_target_level(&mut self, level: f32) {
        if level != self.target_level {
            self.target_level = level;
            self.level_moving = true;
        }
    }

    pub fn is_silent(&self) -> bool {
        self.level == 0.0
    }

    pub fn tune_when_silent(&mut self, frequency: f32, sample_rate: f32) {
        self.pair_pending = false;
        let increment = frequency.max(0.01).min(sample_rate * 0.45) / sample_rate;
        self.phase_step = (increment * PHASE_SCALE) as u32;
        self.pitch_moving = true;
    }

    #[cfg(test)]
    fn pitch_increment(&self) -> f32 {
        self.phase_step as f32 * PHASE_SCALE_INVERSE
    }
    #[cfg(test)]
    fn selected_increment(&self) -> f32 {
        self.selection_step as f32 * PHASE_SCALE_INVERSE
    }

    #[cfg(test)]
    pub fn tick(
        &mut self,
        engine: &Engine,
        memory: &Memory,
        frequency: f32,
        sample_rate: f32,
        level: f32,
        frame: usize,
    ) -> f32 {
        self.prepare(frequency, sample_rate, level);
        self.tick_prepared(engine, memory, frame)
    }

    // Monotonic integer tracking cannot leave the interval between its current
    // and target step. Strict bounds preserve the existing refresh thresholds.
    fn selection_covers_target(&self) -> bool {
        self.selection_high != 0
            && self.phase_step.min(self.target_step) > self.selection_low
            && self.phase_step.max(self.target_step) < self.selection_high
    }
    // One two-sample exponential endpoint, with a
    // linear midpoint. Keep the existing per-sample level slew unchanged.
    #[inline(always)]
    fn advance_pitch_step(&mut self) {
        if self.pitch_moving {
            if self.pair_pending {
                self.phase_step = self.pair_end;
                self.pair_pending = false;
            } else {
                let delta = (self.target_step as i32).wrapping_sub(self.phase_step as i32);
                let rounded =
                    ((delta as i64 * self.pitch_pair_slew as i64 + (1i64 << 29)) >> 30) as i32;
                let change = if rounded == 0 {
                    delta.signum()
                } else {
                    rounded
                };
                self.pair_end = self.phase_step.wrapping_add(change as u32);
                self.phase_step = self.phase_step.wrapping_add((change / 2) as u32);
                self.pair_pending = true;
            }
            self.pitch_moving = self.phase_step != self.target_step || self.pair_pending;
        }
    }
    #[inline(always)]
    fn advance_pitch(&mut self, engine: &Engine) {
        if self.pitch_moving {
            self.advance_pitch_step();
            if self.selection_high == 0
                || self.phase_step >= self.selection_high
                || self.phase_step <= self.selection_low
            {
                let increment = self.phase_step as f32 * PHASE_SCALE_INVERSE;
                (self.playback_level, self.playback_blend) =
                    engine.layout.playback_levels(increment);
                self.selection_low = ((self.phase_step as u64 * SELECTION_LOW_Q31) >> 31) as u32;
                self.selection_high = ((self.phase_step as u64 * SELECTION_HIGH_Q31
                    + ((1u64 << 31) - 1))
                    >> 31) as u32;
                #[cfg(test)]
                {
                    self.selection_step = self.phase_step;
                }
            }
        }
    }

    pub fn tick_prepared(&mut self, engine: &Engine, memory: &Memory, frame: usize) -> f32 {
        #[cfg(test)]
        tests::count_oscillator_tick();
        self.advance_pitch(engine);
        if self.level_moving {
            let previous = self.level;
            self.level += self.slew * (self.target_level - self.level);
            self.level_moving = self.level != previous;
        }
        // All voices see the same fade time for this audio frame. When a fade
        // ends within the block, later frames use only the new bank.
        let fading = engine.fade > 0 && frame < engine.fade;
        let bank = if engine.fade > 0 && !fading {
            1 - engine.active
        } else {
            engine.active
        };
        let old = engine.voice_sample(
            memory,
            bank,
            self.phase,
            self.playback_level,
            self.playback_blend,
        );
        let sample = if fading {
            let blend = 1.0 - (engine.fade - frame) as f32 / engine.fade_length as f32;
            let new = engine.voice_sample(
                memory,
                1 - engine.active,
                self.phase,
                self.playback_level,
                self.playback_blend,
            );
            old + (new - old) * blend
        } else {
            old
        };
        self.phase = self.phase.wrapping_add(self.phase_step);
        sample * self.level * 5.0
    }

    /// Fast path for constant level and a fully open gate, including moving pitch. Table slices are
    /// rebound each block so publication can never leave a stale bank reference.
    pub fn render_open_gate(
        &mut self,
        engine: &Engine,
        memory: &Memory,
        output: &mut [f32],
    ) -> bool {
        if self.level_moving || engine.fade != 0 || !engine.ready[engine.active & 1] {
            return false;
        }
        if self.pitch_moving {
            if !self.selection_covers_target() {
                return self.render_moving_pitch(engine, memory, output);
            }
            return self.render_fixed_selection::<true>(engine, memory, output);
        }
        self.render_fixed_selection::<false>(engine, memory, output)
    }
    /// Render into a cleared voice buffer using the same bound-table loop as
    /// sustained playback, then apply a precomputed group envelope. Stop before
    /// the first zero gate, matching the scalar path's phase/pitch advancement.
    pub fn render_enveloped(
        &mut self,
        engine: &Engine,
        memory: &Memory,
        output: &mut [f32],
        gates: &[f32],
    ) -> Option<f32> {
        let gates = gates.get(..output.len())?;
        let audible = gates
            .iter()
            .position(|&gate| gate == 0.0)
            .unwrap_or(gates.len());
        if !self.render_open_gate(engine, memory, output.get_mut(..audible)?) {
            return None;
        }
        let mut power = 0.0;
        for (sample, &gate) in output.iter_mut().zip(gates).take(audible) {
            *sample *= gate;
            power += gate * gate;
        }
        Some(power)
    }
    fn render_fixed_selection<const TRACK: bool>(
        &mut self,
        engine: &Engine,
        memory: &Memory,
        output: &mut [f32],
    ) -> bool {
        let table_index = (engine.active & 1) * (engine.layout.count + 1) + self.playback_level;
        let Some(rich) = memory.get(table_index / 2) else {
            return false;
        };
        let gain = self.level * 5.0;
        let blend = self.playback_blend;
        let simple = blend == 0.0 || self.playback_level + 1 == engine.layout.count;
        // Resolve table storage and interpolation mode outside the sample loop.
        if simple {
            for sample in output {
                if TRACK {
                    self.advance_pitch_step();
                }
                let index = (self.phase >> 24) as usize;
                let fraction = (self.phase & 0x00ff_ffff) as f32 / 16_777_216.0;
                let value = rich.interpolate(
                    table_index & 1 != 0,
                    index,
                    (index + 1) & (SIZE - 1),
                    fraction,
                );
                *sample += value * gain;
                self.phase = self.phase.wrapping_add(self.phase_step);
                #[cfg(test)]
                if TRACK {
                    tests::count_moving_pitch_tick();
                } else {
                    tests::count_steady_tick();
                }
            }
        } else {
            let Some(lean) = memory.get((table_index + 1) / 2) else {
                return false;
            };
            for sample in output {
                if TRACK {
                    self.advance_pitch_step();
                }
                let index = (self.phase >> 24) as usize;
                let next = (index + 1) & (SIZE - 1);
                let fraction = (self.phase & 0x00ff_ffff) as f32 / 16_777_216.0;
                let a = rich.interpolate(table_index & 1 != 0, index, next, fraction);
                let b = lean.interpolate((table_index + 1) & 1 != 0, index, next, fraction);
                *sample += (a + (b - a) * blend) * gain;
                self.phase = self.phase.wrapping_add(self.phase_step);
                #[cfg(test)]
                if TRACK {
                    tests::count_moving_pitch_tick();
                } else {
                    tests::count_steady_tick();
                }
            }
        }
        true
    }
    // Gate, level, bank readiness and edit-fade decisions were made by the caller.
    // Bind tables once and refresh only when pitch selects a different level.
    fn render_moving_pitch(
        &mut self,
        engine: &Engine,
        memory: &Memory,
        output: &mut [f32],
    ) -> bool {
        let base = (engine.active & 1) * (engine.layout.count + 1);
        let Some(first) = memory.get(base / 2) else {
            return false;
        };
        if memory.get((base + engine.layout.count - 1) / 2).is_none() {
            return false;
        }
        let gain = self.level * 5.0;
        let mut selected = usize::MAX;
        let mut rich = first;
        let mut lean = first;
        let mut rich_imaginary = false;
        let mut lean_imaginary = false;
        for sample in output {
            self.advance_pitch(engine);
            if self.playback_level != selected {
                selected = self.playback_level;
                let index = base + selected;
                rich = memory.get(index / 2).unwrap_or(first);
                lean = memory.get((index + 1) / 2).unwrap_or(rich);
                rich_imaginary = index & 1 != 0;
                lean_imaginary = (index + 1) & 1 != 0;
            }
            let index = (self.phase >> 24) as usize;
            let next = (index + 1) & (SIZE - 1);
            let fraction = (self.phase & 0x00ff_ffff) as f32 / 16_777_216.0;
            let a = rich.interpolate(rich_imaginary, index, next, fraction);
            let value = if self.playback_blend == 0.0 || selected + 1 == engine.layout.count {
                a
            } else {
                let b = lean.interpolate(lean_imaginary, index, next, fraction);
                a + (b - a) * self.playback_blend
            };
            *sample += value * gain;
            self.phase = self.phase.wrapping_add(self.phase_step);
            #[cfg(test)]
            tests::count_moving_pitch_tick();
        }
        true
    }
}

#[cfg(test)]
pub fn oscillator_tick_count() -> usize {
    tests::oscillator_tick_count()
}

#[cfg(test)]
pub fn playback_work_counts() -> (usize, usize) {
    tests::playback_work_counts()
}

// Playback banks stay on the audio side; all rebuild scratch belongs to the
// queued/running worker job.
pub const PLAYBACK_BLOCK_COUNT: usize = DEFAULT_TABLES + 1;
#[derive(Clone, Copy)]
pub struct BuildJob {
    wave: Wave,
    completed: bool,
    initialized: bool,
    result: [[f32; SIZE]; DEFAULT_TABLES + 1],
    spectrum: [Complex; SIZE],
    scratch: [Complex; SIZE],
    twiddles: [Complex; SIZE],
}
impl BuildJob {
    pub fn initialize(
        p: &mut distingnt_api::api::plugin::RegionInitializer<'_, Self>,
        count: usize,
    ) -> Option<()> {
        distingnt_api::initialize_region!(
            p,
            BuildJob {
                wave: Wave::blank(count),
                completed: false,
                initialized: false,
            },
            arrays {},
            regions {
                result: |field| field.initialize_each(|row, _| row.fill(0.0)),
                spectrum: |field| field.initialize_each(|value, _| Complex::initialize(value)),
                scratch: |field| field.initialize_each(|value, _| Complex::initialize(value)),
                twiddles: |field| field.initialize_each(|value, _| Complex::initialize(value)),
            }
        );
        Some(())
    }
    /// Rebuild the complete table set on the background worker.
    /// Two audio tables are packed into each inverse complex transform.
    pub fn build(&mut self) {
        self.completed = false;
        if !self.initialized {
            for (i, value) in self.twiddles.iter_mut().take(SIZE / 2).enumerate() {
                let angle = core::f32::consts::TAU * i as f32 / SIZE as f32;
                *value = Complex {
                    re: sine(angle + core::f32::consts::FRAC_PI_2),
                    im: -sine(angle),
                };
            }
            for harmonic in 0..=SIZE / 2 {
                let ratio = harmonic as f32 / 4.0;
                let gain = 1.0 / (1.0 + ratio * ratio * ratio * ratio);
                let slot = &mut self.twiddles[SIZE / 2 + harmonic / 2];
                if harmonic & 1 == 0 {
                    slot.re = gain;
                } else {
                    slot.im = gain;
                }
            }
            self.initialized = true;
        }
        if self.wave.warp_amount != 0 {
            prepare_geometry(&self.wave, &mut self.twiddles);
        }
        let mut cursor = 0;
        for (i, value) in self.spectrum.iter_mut().enumerate() {
            let phase = i as f32 / SIZE as f32;
            let sample = if self.wave.smoothing > 0 || self.wave.warp_amount == 0 {
                self.wave.sample_unwarped(phase)
            } else {
                let (segment, t) =
                    cached_segment_at(&self.wave, &self.twiddles, phase, &mut cursor);
                self.wave.segment_sample(segment, t)
            };
            *value = Complex {
                re: sample,
                im: 0.0,
            };
            self.result[DEFAULT_TABLES][i] = sample;
        }
        fft(&mut self.spectrum, &self.twiddles, false);
        if self.wave.smoothing > 0 {
            let smoothing = self.wave.smoothing as f32 * 0.01;
            for (k, value) in self.scratch.iter_mut().enumerate() {
                let harmonic = k.min(SIZE - k);
                let slot = self.twiddles[(SIZE / 2 + harmonic / 2).min(SIZE - 1)];
                let filtered = if harmonic & 1 == 0 { slot.re } else { slot.im };
                let gain = 1.0 + smoothing * (filtered - 1.0);
                *value = Complex {
                    re: self.spectrum[k].re * gain,
                    im: self.spectrum[k].im * gain,
                };
            }
            fft(&mut self.scratch, &self.twiddles, true);
            for _ in 0..2 {
                self.anchor_boundaries();
            }
            self.scratch[0].re = self.wave.boundary_level(0);
            for (to, from) in self.result[DEFAULT_TABLES].iter_mut().zip(&self.scratch) {
                *to = from.re;
            }
            if self.wave.warp_amount != 0 {
                let preview = &self.result[DEFAULT_TABLES];
                let mut cursor = 0;
                for (i, value) in self.scratch.iter_mut().enumerate() {
                    let (segment, t) = cached_segment_at(
                        &self.wave,
                        &self.twiddles,
                        i as f32 / SIZE as f32,
                        &mut cursor,
                    );
                    let phase = (segment as f32 + t) / self.wave.count as f32;
                    let fixed = (phase * 4_294_967_296.0) as u32;
                    let index = (fixed >> 24) as usize;
                    let fraction = (fixed & 0x00ff_ffff) as f32 / 16_777_216.0;
                    *value = Complex {
                        re: preview[index]
                            + (preview[(index + 1) & (SIZE - 1)] - preview[index]) * fraction,
                        im: 0.0,
                    };
                }
            }
            fft(&mut self.scratch, &self.twiddles, false);
            self.spectrum.copy_from_slice(&self.scratch);
        }
        let layout = TableLayout::new(DEFAULT_TABLES);
        for level in (0..DEFAULT_TABLES).step_by(2) {
            let cutoff = layout.cutoff(level);
            let second_cutoff = layout.cutoff(level + 1);
            let has_second = level + 1 < DEFAULT_TABLES;
            for (k, value) in self.scratch.iter_mut().enumerate() {
                let harmonic = k.min(SIZE - k);
                let first = if harmonic <= cutoff && harmonic < SIZE / 2 {
                    self.spectrum[k]
                } else {
                    Complex::default()
                };
                let second = if has_second && harmonic <= second_cutoff && harmonic < SIZE / 2 {
                    self.spectrum[k]
                } else {
                    Complex::default()
                };
                *value = Complex {
                    re: first.re - second.im,
                    im: first.im + second.re,
                };
            }
            fft(&mut self.scratch, &self.twiddles, true);
            let Some(first) = self.result.get_mut(level) else {
                return;
            };
            for (to, from) in first.iter_mut().zip(&self.scratch) {
                *to = from.re;
            }
            if has_second {
                let Some(second) = self.result.get_mut(level + 1) else {
                    return;
                };
                for (to, from) in second.iter_mut().zip(&self.scratch) {
                    *to = from.im;
                }
            }
        }
        self.completed = true;
    }
    fn anchor_boundaries(&mut self) {
        let n = self.wave.count.clamp(2, MAX_SEGMENTS);
        for (i, error) in self.spectrum.iter_mut().take(n).enumerate() {
            let position = i as f32 * SIZE as f32 / n as f32;
            let index = position as usize & (SIZE - 1);
            let fraction = position - index as f32;
            let actual = self.scratch[index].re
                + (self.scratch[(index + 1) & (SIZE - 1)].re - self.scratch[index].re) * fraction;
            error.re = self.wave.boundary_level(i) - actual;
        }
        let dc = self
            .spectrum
            .iter()
            .take(n)
            .map(|error| error.re)
            .sum::<f32>()
            / n as f32;
        for value in &mut self.scratch {
            value.re += dc;
        }
        for k in 1..=n / 2 {
            // Evaluate the boundary analysis weights in the worker, avoiding
            // a separate persistent cache for every segment/harmonic pair.
            let angle = core::f32::consts::TAU * k as f32 / n as f32;
            let (wc, ws) = (sine(angle + core::f32::consts::FRAC_PI_2), sine(angle));
            let (mut cosine, mut sinusoid) = (1.0, 0.0);
            let (mut a, mut b) = (0.0, 0.0);
            for error in self.spectrum.iter().take(n) {
                a += error.re * cosine;
                b += error.re * sinusoid;
                let next = cosine * wc - sinusoid * ws;
                sinusoid = sinusoid * wc + cosine * ws;
                cosine = next;
            }
            let scale = if n % 2 == 0 && k == n / 2 { 1.0 } else { 2.0 } / n as f32;
            let (a, b) = (a * scale, b * scale);
            let mut phase = 0;
            for value in &mut self.scratch {
                let basis = self.twiddles[phase & (SIZE / 2 - 1)];
                let correction = a * basis.re - b * basis.im;
                value.re += if phase < SIZE / 2 {
                    correction
                } else {
                    -correction
                };
                phase = (phase + k) & (SIZE - 1);
            }
        }
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub struct Engine {
    layout: TableLayout,
    requested: Wave,
    waves: [Wave; 2],
    active: usize,
    dirty: bool,
    fade: usize,
    fade_length: usize,
    ready: [bool; 2],
}

impl Engine {
    #[inline(always)]
    pub fn initialize(
        destination: &mut distingnt_api::api::plugin::RegionInitializer<'_, Self>,
        count: usize,
        sample_rate: u32,
    ) {
        let wave = Wave::new(count);
        distingnt_api::initialize_region!(
            destination,
            Engine {
                layout: TableLayout::new(DEFAULT_TABLES),
                requested: wave,
                active: 0,
                dirty: true,
                fade: 0,
                fade_length: (sample_rate as usize / 200).max(1),
                ready: [false; 2],
            },
            arrays { waves: |_| wave }
        );
    }

    /// Update the preallocated requested shape without a callback-local snapshot.
    /// Active playback and an in-progress build retain their separate copies.
    #[inline(always)]
    pub fn update_request(&mut self, update: impl FnOnce(&mut Wave)) {
        update(&mut self.requested);
        let latest = if self.fade > 0 {
            1 - self.active
        } else {
            self.active
        };
        self.dirty = !self.ready[latest & 1] || self.requested != self.waves[latest & 1];
    }

    /// Poll a completed result and snapshot the latest request for a new job.
    /// Called only while the shared job slot is exclusively reserved by audio.
    pub fn service_job(&mut self, job: &mut BuildJob, memory: &mut Memory) -> bool {
        if self.fade != 0 {
            return false;
        }
        if job.completed {
            if memory.len() < PLAYBACK_BLOCK_COUNT {
                return false;
            }
            let base = (1 - self.active) * (DEFAULT_TABLES + 1);
            for (level, source) in job.result.iter().enumerate() {
                let index = base + level;
                let Some(block) = memory.get_mut(index / 2) else {
                    return false;
                };
                for (destination, &sample) in block.data.iter_mut().zip(source) {
                    if index & 1 == 0 {
                        destination.re = sample;
                    } else {
                        destination.im = sample;
                    }
                }
            }
            let inactive = (1 - self.active) & 1;
            self.waves[inactive] = job.wave;
            self.ready[inactive] = true;
            self.fade = self.fade_length;
            self.dirty = self.requested != job.wave;
            job.completed = false;
            return false;
        }
        if !self.dirty {
            return false;
        }
        job.wave = self.requested;
        true
    }

    fn lookup(&self, memory: &Memory, table_index: usize, phase: u32) -> f32 {
        let index = (phase >> 24) as usize;
        let fraction = (phase & 0x00ff_ffff) as f32 / 16_777_216.0;
        let Some(block) = memory.get(table_index / 2) else {
            return 0.0;
        };
        block.interpolate(
            table_index & 1 != 0,
            index,
            (index + 1) & (SIZE - 1),
            fraction,
        )
    }

    fn voice_sample(
        &self,
        memory: &Memory,
        bank: usize,
        phase: u32,
        playback_level: usize,
        playback_blend: f32,
    ) -> f32 {
        if !self.ready[bank & 1] {
            return 0.0;
        }
        let index = (bank & 1) * (self.layout.count + 1) + playback_level;
        let rich = self.lookup(memory, index, phase);
        if playback_blend == 0.0 || playback_level + 1 == self.layout.count {
            return rich;
        }
        rich + (self.lookup(memory, index + 1, phase) - rich) * playback_blend
    }
    pub fn advance_time(&mut self, frames: usize) {
        if self.fade > 0 {
            if frames >= self.fade {
                self.fade = 0;
                self.active = 1 - self.active;
            } else {
                self.fade -= frames;
            }
        }
    }

    /// Both banks are evaluated at the same horizontal phase during a fade,
    /// including when their segment boundaries occupy different positions.
    pub fn editor_phase_sample(&self, memory: &Memory, phase: f32) -> f32 {
        let sample = |bank: usize| {
            let wave = &self.waves[bank & 1];
            if wave.smoothing == 0 {
                wave.sample(phase)
            } else {
                let phase = if phase >= 1.0 {
                    0
                } else {
                    (wave.unwarped_phase(phase) * 4_294_967_296.0) as u32
                };
                self.lookup(
                    memory,
                    (bank & 1) * (self.layout.count + 1) + self.layout.count,
                    phase,
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
}

#[cfg(test)]
#[path = "dsp_tests.rs"]
mod tests;
