//! Waveform math shared by all KickLab instances on the loaded factory.
use core::f32::consts::LOG2_E;
use core::mem::size_of;
use distingnt_api::api::{
    math::exp2,
    plugin::{StaticDramView, StaticMemory},
};

const STEPS: usize = 1024;
const SIZE: usize = STEPS + 1;
const DRIVE_LIMIT: f32 = 8.0;
pub const BYTES: u32 = (2 * SIZE * size_of::<f32>()) as u32;

#[derive(Clone, Copy)]
pub struct Lookup<'a> {
    drive: &'a [f32; SIZE],
    pitch: &'a [f32; SIZE],
}

pub fn initialize(mut memory: StaticMemory<'_>) {
    if let Some(values) = memory.dram_elements_mut::<f32>() {
        fill(values);
    }
}

fn fill(values: &mut [f32]) {
    let Some((drive, pitch)) = values.split_at_mut_checked(SIZE) else {
        return;
    };
    if pitch.len() != SIZE {
        return;
    }
    for (index, (drive, pitch)) in drive.iter_mut().zip(pitch).enumerate() {
        let fraction = index as f32 / STEPS as f32;
        *drive = drive_exact(fraction * DRIVE_LIMIT);
        *pitch = exp2(fraction);
    }
}

impl<'a> Lookup<'a> {
    pub fn from_dram(memory: StaticDramView<'a>) -> Option<Self> {
        Self::from_elements(memory.elements()?)
    }

    fn from_elements(values: &'a [f32]) -> Option<Self> {
        let (drive, pitch) = values.split_at_checked(SIZE)?;
        Some(Self {
            drive: drive.try_into().ok()?,
            pitch: pitch.try_into().ok()?,
        })
    }
}

fn interpolate(table: &[f32; SIZE], position: f32) -> f32 {
    let index = position as usize;
    match table.get(index..).and_then(|tail| tail.first_chunk::<2>()) {
        Some(&[first, second]) => first + (second - first) * (position - index as f32),
        None => table[STEPS],
    }
}

fn drive_exact(value: f32) -> f32 {
    let edge = exp2(-2.0 * LOG2_E * value.abs());
    ((1.0 - edge) / (1.0 + edge)).copysign(value)
}

#[inline(never)]
pub fn drive(tables: Option<Lookup<'_>>, value: f32) -> f32 {
    if let Some(tables) = tables {
        let magnitude = value.abs();
        let output = if magnitude >= DRIVE_LIMIT {
            1.0
        } else {
            interpolate(tables.drive, magnitude * (STEPS as f32 / DRIVE_LIMIT))
        };
        output.copysign(value)
    } else {
        drive_exact(value)
    }
}

#[inline(never)]
pub fn pitch_multiplier(tables: Option<Lookup<'_>>, octaves: f32) -> f32 {
    if let Some(tables) = tables {
        // KickLab's pitch sweep is specified in 0..60 semitones. Using a
        // fractional octave table keeps interpolation error near f32 precision.
        if (0.0..=5.0).contains(&octaves) {
            let whole = octaves as u32;
            let fraction = (octaves - whole as f32) * STEPS as f32;
            let integral = f32::from_bits((127 + whole) << 23);
            return integral * interpolate(tables.pitch, fraction);
        }
    }
    exp2(octaves)
}

#[cfg(test)]
pub fn test_tables(buffer: &mut [f32; 2 * SIZE]) -> Lookup<'_> {
    fill(buffer);
    Lookup::from_elements(buffer).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_match_exact_math_across_their_domains() {
        let mut memory = [0.0; 2 * SIZE];
        let tables = Some(test_tables(&mut memory));
        for index in 0..=32_768 {
            let fraction = index as f32 / 32_768.0;
            let value = fraction * 32.0 - 16.0;
            assert!((drive(tables, value) - drive_exact(value)).abs() < 0.000006);
            let octaves = fraction * 5.0;
            let exact = exp2(octaves);
            assert!((pitch_multiplier(tables, octaves) / exact - 1.0).abs() <= 0.00000024);
        }
    }
}
