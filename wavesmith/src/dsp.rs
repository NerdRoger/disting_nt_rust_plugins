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
#[cfg(test)]
const LEVELS: usize = DEFAULT_TABLES;
#[cfg(test)]
const TABLES_PER_BANK: usize = LEVELS + 1;
// Pack two real-valued tables into each block's real/imaginary lanes.
#[cfg(test)]
const BANK_BLOCKS: usize = TABLES_PER_BANK;
// Each paired inverse FFT produces at most two audio tables.
const TABLES_PER_STEP: usize = 2;
const TABLE_PAIR_STEPS: usize = 4;
const HARMONICS_PER_STEP: usize = 4;
const SAMPLES_PER_STEP: usize = 64;
const SAMPLE_STEPS: usize = SIZE / SAMPLES_PER_STEP;
const SOURCE_FFT_STAGE: usize = 1 + SAMPLE_STEPS;
const SOURCE_STEPS: usize = SOURCE_FFT_STAGE + 2;
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
#[cfg(test)]
const TWIDDLES: usize = BANK_BLOCKS + 2;

#[cfg(test)]
const BUILD_STEPS: usize = LEVELS.div_ceil(TABLES_PER_STEP) * TABLE_PAIR_STEPS + SOURCE_STEPS;

#[derive(Clone, Copy)]
struct TableLayout {
    count: usize,
    cutoffs: [u8; MAX_TABLES],
}

impl TableLayout {
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
    pub fn new(count: usize) -> Self {
        let count = count.clamp(2, MAX_SEGMENTS);
        let mut wave = Self {
            count,
            smoothing: 0,
            warp_position: WARP_POSITION_DEFAULT,
            warp_amount: 0,
            fold_limit: DEFAULT_FOLD_LIMIT,
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
        let (index, t) = self.segment_at(phase);
        self.segment_sample(index, t)
    }

    pub fn boundary_position(&self, boundary: usize) -> f32 {
        #[cfg(test)]
        tests::count_boundary_position();
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
fn add_harmonic(data: &mut [Complex; SIZE], twiddles: &[Complex; SIZE], k: usize, a: f32, b: f32) {
    let mut phase = 0;
    for value in data {
        let basis = twiddles[phase & (SIZE / 2 - 1)];
        let correction = a * basis.re - b * basis.im;
        value.re += if phase < SIZE / 2 {
            correction
        } else {
            -correction
        };
        phase = (phase + k) & (SIZE - 1);
    }
}

// Cache the boundary analysis basis once per specification. Two independent
// blocks hold up to 16 harmonics x 32 boundaries, without a large State/stack.
fn prepare_boundary_basis(n: usize, basis: &mut [Block; 2]) {
    let n = n.clamp(2, MAX_SEGMENTS);
    for k in 1..=n / 2 {
        let angle = core::f32::consts::TAU * k as f32 / n as f32;
        let (wc, ws) = (sine(angle + core::f32::consts::FRAC_PI_2), sine(angle));
        let (mut cosine, mut sinusoid) = (1.0, 0.0);
        for i in 0..n {
            let index = (k - 1) * n + i;
            basis[(index / SIZE) & 1].data[index & (SIZE - 1)] = Complex {
                re: cosine,
                im: sinusoid,
            };
            let next = cosine * wc - sinusoid * ws;
            sinusoid = sinusoid * wc + cosine * ws;
            cosine = next;
        }
    }
}

/// Explicit smoothing retains shared boundaries, including the seam. This is
/// independent of oscillator pitch. The final anti-alias filter follows this.
fn anchor_boundary_step(
    wave: &Wave,
    data: &mut [Complex; SIZE],
    twiddles: &[Complex; SIZE],
    basis: &[Block; 2],
    errors: &mut [Complex; SIZE],
    step: usize,
) {
    let n = wave.count.clamp(2, MAX_SEGMENTS);
    if step == 0 {
        // The source spectrum is no longer needed after the smoothing IFFT.
        // Borrow it to retain errors across callbacks without another buffer.
        for (i, error) in errors.iter_mut().take(n).enumerate() {
            let position = i as f32 * SIZE as f32 / n as f32;
            let index = position as usize & (SIZE - 1);
            let fraction = position - index as f32;
            let actual =
                data[index].re + (data[(index + 1) & (SIZE - 1)].re - data[index].re) * fraction;
            let target = wave.boundary_level(i);
            error.re = target - actual;
        }
        let dc = errors.iter().take(n).map(|error| error.re).sum::<f32>() / n as f32;
        for value in data.iter_mut() {
            value.re += dc;
        }
    } else {
        let first = 1 + (step - 1) * HARMONICS_PER_STEP;
        for k in first..(first + HARMONICS_PER_STEP).min(n / 2 + 1) {
            let (mut a, mut b) = (0.0, 0.0);
            for (i, error) in errors.iter().take(n).enumerate() {
                let index = (k - 1) * n + i;
                let weight = basis[(index / SIZE) & 1].data[index & (SIZE - 1)];
                a += error.re * weight.re;
                b += error.re * weight.im;
            }
            let scale = if n % 2 == 0 && k == n / 2 { 1.0 } else { 2.0 } / n as f32;
            add_harmonic(data, twiddles, k, a * scale, b * scale);
        }
    }
}

fn correction_steps(count: usize) -> usize {
    1 + (count.clamp(2, MAX_SEGMENTS) / 2).div_ceil(HARMONICS_PER_STEP)
}

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

fn audio_table_stage(wave: &Wave) -> usize {
    if wave.smoothing > 0 {
        // Sampling, source FFT halves, filter, inverse FFT halves, correction,
        // preview storage, corrected FFT halves, and spectrum copy.
        SOURCE_STEPS
            + 7
            + 2 * correction_steps(wave.count)
            + if wave.warp_amount != 0 {
                SAMPLE_STEPS
            } else {
                0
            }
    } else {
        SOURCE_STEPS
    }
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

#[cfg(test)]
fn fft(data: &mut [Complex; SIZE], twiddles: &[Complex; SIZE], inverse: bool) {
    #[cfg(test)]
    tests::count_fft(inverse);
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
fn fft_half(data: &mut [Complex; SIZE], twiddles: &[Complex; SIZE], inverse: bool, second: bool) {
    #[cfg(test)]
    {
        tests::count_fft_half();
        if second {
            tests::count_fft(inverse);
        }
    }
    if !second {
        for i in 0..SIZE {
            let j = (i.reverse_bits() >> (usize::BITS - 8)) & (SIZE - 1);
            if i < j {
                data.swap(i, j);
            }
        }
    }
    let mut width = if second { 32 } else { 2 };
    let limit = if second { SIZE } else { 16 };
    while width <= limit {
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
    if inverse && second {
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
    pub fn initialize(destination: &mut distingnt_api::api::plugin::RegionInitializer<'_, Self>) {
        distingnt_api::initialize_region!(
            destination,
            Block {
                data: [Complex { re: 0.0, im: 0.0 }; SIZE],
            }
        );
    }
    #[cfg(test)]
    pub const ZERO: Self = Self {
        data: [Complex { re: 0.0, im: 0.0 }; SIZE],
    };
}

// The framework constructs these small SRAM blocks independently. Keeping
// large buffers outside State avoids large temporary return values on the
// firmware stack. Each table uses one real or imaginary lane in a packed block.
pub const fn blocks(tables: usize) -> usize {
    tables + 6
}
#[cfg(test)]
const BLOCKS: usize = blocks(DEFAULT_TABLES);
pub type Memory = [Block];

fn store_table(
    banks: &mut [Block],
    index: usize,
    source: &[Complex; SIZE],
    imaginary: bool,
) -> Option<()> {
    let destination = banks.get_mut(index / 2)?;
    for (to, from) in destination.data.iter_mut().zip(source) {
        let value = if imaginary { from.im } else { from.re };
        if index & 1 == 0 {
            to.re = value;
        } else {
            to.im = value;
        }
    }
    Some(())
}

#[derive(Clone, Copy)]
struct CvFactor {
    bus: i16,
    voltage: f32,
    factor: f32,
}

/// At most one distinct input per voice; retain exact inputs between callbacks.
#[derive(Clone, Copy)]
pub struct CvPitchCache {
    entries: [CvFactor; super::VOICE_COUNT],
    used: usize,
}

impl CvPitchCache {
    pub const fn new() -> Self {
        Self {
            entries: [CvFactor {
                bus: -1,
                voltage: f32::INFINITY,
                factor: 1.0,
            }; super::VOICE_COUNT],
            used: 0,
        }
    }
    pub fn begin_block(&mut self) {
        self.used = 0;
    }
    pub fn factor(&mut self, bus: i16, voltage: f32) -> f32 {
        if voltage == 0.0 {
            return 1.0;
        }
        let position = self
            .entries
            .iter()
            .take(self.used)
            .position(|entry| entry.bus == bus);
        let index = position.unwrap_or(self.used);
        let Some(entry) = self.entries.get_mut(index) else {
            return distingnt_api::api::math::exp2(voltage);
        };
        if position.is_none() {
            self.used += 1;
        }
        if entry.bus != bus || entry.voltage != voltage {
            entry.bus = bus;
            entry.voltage = voltage;
            entry.factor = distingnt_api::api::math::exp2(voltage);
            #[cfg(test)]
            tests::count_frequency_conversion();
        }
        entry.factor
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
    pitch_slew: i32,
    #[cfg(test)]
    selection_step: u32,
    cached_octave: f32,
    cached_frequency: f32,
    cached_rate: f32,
    prepared_frequency: f32,
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
            pitch_slew: 0,
            #[cfg(test)]
            selection_step: 0,
            cached_octave: f32::INFINITY,
            cached_frequency: 0.0,
            cached_rate: 0.0,
            prepared_frequency: 0.0,
            target_step: 0,
            target_level: 0.0,
            slew: 0.0,
            phase_step: 0,
            pitch_moving: false,
            level_moving: false,
        }
    }
    #[cfg(test)]
    pub fn prepared_frequency_for_test(&self) -> f32 {
        self.prepared_frequency
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

    pub fn cv_frequency(&mut self, tuning: f32, factor: f32) -> f32 {
        // exp2(tuning + CV) = exp2(tuning) * exp2(CV). The tuning factor is
        // cached per voice; the input factor is shared even for detuned voices.
        (self.frequency(tuning) * factor).clamp(440.0 / 65536.0, 440.0 * 65536.0)
    }

    pub fn prepare(&mut self, frequency: f32, sample_rate: f32, level: f32) {
        if frequency != self.prepared_frequency || sample_rate != self.cached_rate {
            self.prepared_frequency = frequency;
            if sample_rate != self.cached_rate {
                self.cached_rate = sample_rate;
                self.slew = (1000.0 / sample_rate).min(1.0);
                self.pitch_slew = ((self.slew * PITCH_SLEW_SCALE) as i32).max(1);
            }
            let increment = frequency.max(0.01).min(sample_rate * 0.45) / sample_rate;
            self.target_step = (increment * PHASE_SCALE) as u32;
            self.pitch_moving = self.phase_step != self.target_step
                || self.selection_high == 0
                || self.phase_step >= self.selection_high
                || self.phase_step <= self.selection_low;
        }
        if level != self.target_level {
            self.target_level = level;
            self.level_moving = true;
        }
    }

    pub fn tune_when_silent(&mut self, frequency: f32, sample_rate: f32) {
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

    // Exponential pitch tracking in phase-step units. Current/target steps are
    // bounded by 0.45 * 2^32, so their signed difference and Q30 product fit.
    #[inline(always)]
    fn advance_pitch(&mut self, engine: &Engine) {
        if self.pitch_moving {
            let delta = (self.target_step as i32).wrapping_sub(self.phase_step as i32);
            if delta != 0 {
                let rounded = ((delta as i64 * self.pitch_slew as i64 + (1i64 << 29)) >> 30) as i32;
                // Never stick short of the target: continue in one phase-step
                // units when the rounded exponential adjustment reaches zero.
                let change = if rounded == 0 {
                    delta.signum()
                } else {
                    rounded
                };
                self.phase_step = self.phase_step.wrapping_add(change as u32);
            }
            self.pitch_moving = self.phase_step != self.target_step;
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
            return self.render_moving_pitch(engine, memory, output);
        }
        let table_index = (engine.active & 1) * (engine.layout.count + 1) + self.playback_level;
        let Some(rich) = memory.get(table_index / 2) else {
            return false;
        };
        let read = |block: &Block, imaginary: bool, index: usize, next: usize, fraction: f32| {
            let (left, right) = if imaginary {
                (block.data[index].im, block.data[next].im)
            } else {
                (block.data[index].re, block.data[next].re)
            };
            left + (right - left) * fraction
        };
        let gain = self.level * 5.0;
        let blend = self.playback_blend;
        let simple = blend == 0.0 || self.playback_level + 1 == engine.layout.count;
        // Resolve table storage and interpolation mode outside the sample loop.
        if simple {
            for sample in output {
                let index = (self.phase >> 24) as usize;
                let fraction = (self.phase & 0x00ff_ffff) as f32 / 16_777_216.0;
                let value = read(
                    rich,
                    table_index & 1 != 0,
                    index,
                    (index + 1) & (SIZE - 1),
                    fraction,
                );
                *sample += value * gain;
                self.phase = self.phase.wrapping_add(self.phase_step);
                #[cfg(test)]
                tests::count_steady_tick();
            }
        } else {
            let Some(lean) = memory.get((table_index + 1) / 2) else {
                return false;
            };
            for sample in output {
                let index = (self.phase >> 24) as usize;
                let next = (index + 1) & (SIZE - 1);
                let fraction = (self.phase & 0x00ff_ffff) as f32 / 16_777_216.0;
                let a = read(rich, table_index & 1 != 0, index, next, fraction);
                let b = read(lean, (table_index + 1) & 1 != 0, index, next, fraction);
                *sample += (a + (b - a) * blend) * gain;
                self.phase = self.phase.wrapping_add(self.phase_step);
                #[cfg(test)]
                tests::count_steady_tick();
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
        let read = |block: &Block, imaginary: bool, index: usize, next: usize, fraction: f32| {
            let (a, b) = if imaginary {
                (block.data[index].im, block.data[next].im)
            } else {
                (block.data[index].re, block.data[next].re)
            };
            a + (b - a) * fraction
        };
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
            let a = read(rich, rich_imaginary, index, next, fraction);
            let value = if self.playback_blend == 0.0 || selected + 1 == engine.layout.count {
                a
            } else {
                let b = read(lean, lean_imaginary, index, next, fraction);
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
    basis_count: usize,
    sample_segment: usize,
    fade: usize,
    fade_length: usize,
    #[cfg(test)]
    phase: u32,
    #[cfg(test)]
    level: f32,
    #[cfg(test)]
    increment: f32,
    ready: [bool; 2],
    #[cfg(test)]
    playback_level: usize,
    #[cfg(test)]
    playback_blend: f32,
}

impl Engine {
    pub fn new(count: usize, sample_rate: u32) -> Self {
        Self::with_tables(count, sample_rate, DEFAULT_TABLES)
    }

    fn with_tables(count: usize, sample_rate: u32, tables: usize) -> Self {
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
            basis_count: 0,
            sample_segment: 0,
            fade: 0,
            fade_length: (sample_rate as usize / 200).max(1),
            #[cfg(test)]
            phase: 0,
            #[cfg(test)]
            level: 0.0,
            #[cfg(test)]
            increment: 0.0,
            ready: [false; 2],
            #[cfg(test)]
            playback_level: 0,
            #[cfg(test)]
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
        let [spectrum, scratch, twiddles, basis @ ..] = workspace else {
            return;
        };
        let Ok(basis) = <&mut [Block; 2]>::try_from(basis) else {
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
            // The unused half of the twiddle buffer holds the fixed smoothing
            // response for harmonics 0..=128, packed into real/imaginary lanes.
            for harmonic in 0..=SIZE / 2 {
                let ratio = harmonic as f32 / 4.0;
                let gain = 1.0 / (1.0 + ratio * ratio * ratio * ratio);
                let slot = &mut twiddles.data[SIZE / 2 + harmonic / 2];
                if harmonic & 1 == 0 {
                    slot.re = gain;
                } else {
                    slot.im = gain;
                }
            }
            self.initialized = true;
        }
        if self.basis_count != self.building.count {
            prepare_boundary_basis(self.building.count, basis);
            self.basis_count = self.building.count;
        }
        if self.stage == 0 {
            if self.building.warp_amount != 0 {
                prepare_geometry(&self.building, &mut twiddles.data);
            }
            self.sample_segment = 0;
            self.stage = 1;
            return;
        }
        if self.stage < SOURCE_STEPS {
            if self.stage < SOURCE_FFT_STAGE {
                // Smooth and anchor in equal-width segment coordinates, then
                // stretch that completed shape before final anti-alias filtering.
                let mut source = self.building;
                if source.smoothing > 0 {
                    source.warp_amount = 0;
                }
                let first = (self.stage - 1) * SAMPLES_PER_STEP;
                #[cfg(test)]
                tests::count_generated_samples(SAMPLES_PER_STEP);
                for (i, value) in spectrum
                    .data
                    .iter_mut()
                    .enumerate()
                    .skip(first)
                    .take(SAMPLES_PER_STEP)
                {
                    let phase = i as f32 / SIZE as f32;
                    let sample = if source.warp_amount == 0 {
                        source.sample(phase)
                    } else {
                        let (segment, t) = cached_segment_at(
                            &source,
                            &twiddles.data,
                            phase,
                            &mut self.sample_segment,
                        );
                        source.segment_sample(segment, t)
                    };
                    *value = Complex {
                        re: sample,
                        im: 0.0,
                    };
                }
            } else {
                fft_half(
                    &mut spectrum.data,
                    &twiddles.data,
                    false,
                    self.stage == SOURCE_FFT_STAGE + 1,
                );
            }
            self.stage += 1;
            return;
        }
        let audio_stage = audio_table_stage(&self.building);
        if self.stage < audio_stage {
            let correction_start = SOURCE_STEPS + 3;
            let correction_end = correction_start + 2 * correction_steps(self.building.count);
            match self.stage {
                SOURCE_STEPS => {
                    let smoothing = self.building.smoothing as f32 * 0.01;
                    for (k, value) in scratch.data.iter_mut().enumerate() {
                        let harmonic = k.min(SIZE - k);
                        let slot = twiddles.data[SIZE / 2 + harmonic / 2];
                        let filtered_gain = if harmonic & 1 == 0 { slot.re } else { slot.im };
                        let gain = 1.0 + smoothing * (filtered_gain - 1.0);
                        *value = Complex {
                            re: spectrum.data[k].re * gain,
                            im: spectrum.data[k].im * gain,
                        };
                    }
                }
                stage if stage == SOURCE_STEPS + 1 || stage == SOURCE_STEPS + 2 => fft_half(
                    &mut scratch.data,
                    &twiddles.data,
                    true,
                    stage == SOURCE_STEPS + 2,
                ),
                stage if stage < correction_end => {
                    let step = (stage - correction_start) % correction_steps(self.building.count);
                    anchor_boundary_step(
                        &self.building,
                        &mut scratch.data,
                        &twiddles.data,
                        basis,
                        &mut spectrum.data,
                        step,
                    );
                    if stage + 1 == correction_end {
                        scratch.data[0].re = self.building.boundary_level(0);
                    }
                }
                stage if stage == correction_end => {
                    // Save the preview before transforming its buffer.
                    if store_table(
                        banks,
                        (1 - self.active) * tables_per_bank + self.layout.count,
                        &scratch.data,
                        false,
                    )
                    .is_none()
                    {
                        return;
                    }
                }
                stage
                    if self.building.warp_amount != 0 && stage <= correction_end + SAMPLE_STEPS =>
                {
                    let preview = (1 - self.active) * tables_per_bank + self.layout.count;
                    let first = (stage - correction_end - 1) * SAMPLES_PER_STEP;
                    if first == 0 {
                        self.sample_segment = 0;
                    }
                    #[cfg(test)]
                    tests::count_generated_samples(SAMPLES_PER_STEP);
                    for (i, value) in scratch
                        .data
                        .iter_mut()
                        .enumerate()
                        .skip(first)
                        .take(SAMPLES_PER_STEP)
                    {
                        let (segment, t) = cached_segment_at(
                            &self.building,
                            &twiddles.data,
                            i as f32 / SIZE as f32,
                            &mut self.sample_segment,
                        );
                        let phase = (segment as f32 + t) / self.building.count as f32;
                        *value = Complex {
                            re: self.lookup(banks, preview, (phase * 4_294_967_296.0) as u32),
                            im: 0.0,
                        };
                    }
                }
                stage if stage <= audio_stage - 2 => {
                    fft_half(
                        &mut scratch.data,
                        &twiddles.data,
                        false,
                        stage == audio_stage - 2,
                    );
                }
                _ => spectrum.data.copy_from_slice(&scratch.data),
            }
            self.stage += 1;
            return;
        }
        // Each pair gets four dedicated calls: prepare, FFT half 1, FFT half 2,
        // store. Never expose an unfinished pair to playback or the display.
        let pair_step = (self.stage - audio_stage) % TABLE_PAIR_STEPS;
        let level = (self.stage - audio_stage) / TABLE_PAIR_STEPS * TABLES_PER_STEP;
        let has_second = level + 1 < self.layout.count;
        match pair_step {
            0 => {
                let cutoff = self.layout.cutoff(level);
                let second_cutoff = self.layout.cutoff(level + 1);
                for (k, value) in scratch.data.iter_mut().enumerate() {
                    let harmonic = k.min(SIZE - k);
                    let first =
                        if self.layout.count == 1 || (harmonic <= cutoff && harmonic < SIZE / 2) {
                            spectrum.data[k]
                        } else {
                            Complex::default()
                        };
                    let second = if has_second && harmonic <= second_cutoff && harmonic < SIZE / 2 {
                        spectrum.data[k]
                    } else {
                        Complex::default()
                    };
                    // IFFT(A + iB) = a + ib for two real time-domain signals.
                    *value = Complex {
                        re: first.re - second.im,
                        im: first.im + second.re,
                    };
                }
            }
            1 | 2 => fft_half(&mut scratch.data, &twiddles.data, true, pair_step == 2),
            _ => {
                if store_table(
                    banks,
                    (1 - self.active) * tables_per_bank + level,
                    &scratch.data,
                    false,
                )
                .is_none()
                {
                    return;
                }
                if has_second
                    && store_table(
                        banks,
                        (1 - self.active) * tables_per_bank + level + 1,
                        &scratch.data,
                        true,
                    )
                    .is_none()
                {
                    return;
                }
            }
        }
        self.stage += 1;
        if pair_step == 3 && level + TABLES_PER_STEP >= self.layout.count {
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

    #[cfg(test)]
    fn wave_sample(&self, memory: &Memory, bank: usize, phase: u32) -> f32 {
        self.voice_sample(
            memory,
            bank,
            phase,
            self.playback_level,
            self.playback_blend,
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
    #[cfg(test)]
    pub fn show_voice(&mut self, voice: &Oscillator) {
        self.playback_level = voice.playback_level;
        self.playback_blend = voice.playback_blend;
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

    #[cfg(test)]
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
    #[cfg(test)]
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
                // Preview is retained in equal-width coordinates so even
                // sub-sample compressed segments keep their intended anchors.
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
    #[cfg(test)]
    pub fn audio_segment_sample(&self, memory: &Memory, segment: usize, t: f32) -> f32 {
        let wave = &self.waves[self.active & 1];
        let n = wave.count.max(2);
        let phase = if segment + 1 == n && t == 1.0 {
            0
        } else {
            (wave.segment_phase(segment, t) * 4_294_967_296.0) as u32
        };
        self.display_sample(memory, phase)
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
#[path = "dsp_tests.rs"]
mod tests;
