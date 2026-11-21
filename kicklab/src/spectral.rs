use super::{exp2, render_layer, sine, LayerPatch, Lookup, LOG2_E};

pub(crate) fn spectral_workspace_frames(rate: u32) -> usize {
    (2 * rate as usize).next_power_of_two()
}

struct Spectrum {
    hz_per_bin: f32,
    tune: f32,
    bass_inv_width: f32,
    bass_gain: f32,
    snap_center: f32,
    snap_inv_width: f32,
    snap_gain: f32,
    dispersion: f32,
    last_bin: usize,
}

impl Spectrum {
    fn new(controls: &[f32], rate: u32, size: usize) -> Self {
        let hz_per_bin = rate as f32 / size as f32;
        let tune = controls[0];
        let bass_width = 483.012_05 / controls[1];
        let snap_center = controls[3];
        let snap_width = snap_center * controls[4] * 0.01;
        let scale = rate as f32 * 0.199_471_15;
        let bass_edge = tune + 7.0 * bass_width;
        let snap_gain = scale * 0.007 * controls[2] / snap_width;
        let max_hz = if snap_gain > 0.0 {
            bass_edge.max(snap_center + 7.0 * snap_width)
        } else {
            bass_edge
        };
        Self {
            hz_per_bin,
            tune,
            bass_inv_width: 1.0 / bass_width,
            bass_gain: scale * 0.9 / bass_width,
            snap_center,
            snap_inv_width: 1.0 / snap_width,
            snap_gain,
            dispersion: 0.0005 * controls[5] * snap_width,
            last_bin: ((max_hz / hz_per_bin) as usize + 1).min(size / 2 - 1),
        }
    }

    fn bin(&self, index: usize) -> [f32; 2] {
        if index == 0 || index > self.last_bin {
            return [0.0, 0.0];
        }
        let hz = index as f32 * self.hz_per_bin;
        // The spectrum already truncates at seven bandwidths. Apply that
        // limit to each band individually so the broad snap band cannot
        // force thousands of negligible bass exponentials.
        let bass_position = (hz - self.tune) * self.bass_inv_width;
        let bass = if bass_position.abs() <= 7.0 {
            self.bass_gain * exp2(-0.5 * LOG2_E * bass_position * bass_position)
        } else {
            0.0
        };
        if self.snap_gain == 0.0 {
            return [0.0, -bass];
        }
        let snap_position = (hz - self.snap_center) * self.snap_inv_width;
        if snap_position.abs() > 7.0 {
            return [0.0, -bass];
        }
        let snap_squared = snap_position * snap_position;
        let snap = self.snap_gain * exp2(-0.5 * LOG2_E * snap_squared);
        let phase = hz * 0.008 + self.dispersion * snap_squared;
        [snap * sine(phase + 0.25), -bass - snap * sine(phase)]
    }
}

fn pack_spectrum(spectrum: &Spectrum, pairs: &mut [[f32; 2]]) -> Option<()> {
    pairs.fill([0.0, 0.0]);
    let count = pairs.len();
    let shift = usize::BITS - count.trailing_zeros();
    let spacing = 0.5 / count as f32;
    for bin in 1..=spectrum.last_bin.min(count / 2) {
        // Split the real spectrum into even and odd time sequences and pack
        // them into one complex inverse transform. Paired bins share one
        // calculation, since their twiddle angles add to a half turn.
        let mirror = count - bin;
        let [a, b] = spectrum.bin(bin);
        let [c, d] = spectrum.bin(mirror);
        let turns = bin as f32 * spacing;
        let sin = sine(turns);
        let cos = sine(turns + 0.25);
        let real_mean = (a + c) * 0.5;
        let real_diff = (a - c) * 0.5;
        let imag_mean = (b - d) * 0.5;
        let imag_diff = (b + d) * 0.5;
        let odd_real = cos * real_diff - sin * imag_diff;
        let odd_imag = sin * real_diff + cos * imag_diff;
        *pairs.get_mut(bin.reverse_bits() >> shift)? = [real_mean - odd_imag, imag_mean + odd_real];
        if mirror != bin {
            *pairs.get_mut(mirror.reverse_bits() >> shift)? =
                [real_mean + odd_imag, -imag_mean + odd_real];
        }
    }
    Some(())
}

fn inverse_transform(pairs: &mut [[f32; 2]]) -> Option<()> {
    // Combine the first two radix-2 stages. Their twiddles are 1 and i.
    if let [first, second] = pairs {
        let [ar, ai] = *first;
        let [br, bi] = *second;
        *first = [ar + br, ai + bi];
        *second = [ar - br, ai - bi];
        return Some(());
    }
    for [a, b, c, d] in pairs.as_chunks_mut::<4>().0 {
        let [ar, ai] = *a;
        let [br, bi] = *b;
        let [cr, ci] = *c;
        let [dr, di] = *d;
        let sum_ab = [ar + br, ai + bi];
        let diff_ab = [ar - br, ai - bi];
        let sum_cd = [cr + dr, ci + di];
        let rotated_cd = [di - ci, cr - dr];
        *a = [sum_ab[0] + sum_cd[0], sum_ab[1] + sum_cd[1]];
        *b = [diff_ab[0] + rotated_cd[0], diff_ab[1] + rotated_cd[1]];
        *c = [sum_ab[0] - sum_cd[0], sum_ab[1] - sum_cd[1]];
        *d = [diff_ab[0] - rotated_cd[0], diff_ab[1] - rotated_cd[1]];
    }

    const TWIDDLE_ANCHOR: usize = 128;
    let mut width = 8;
    while width <= pairs.len() {
        let half = width / 2;
        let spacing = 1.0 / width as f32;
        let sin_half_step = sine(spacing * 0.5);
        let sin_step = sine(spacing);
        let cos_step = 1.0 - 2.0 * sin_half_step * sin_half_step;
        // Visit each block in memory order. Re-anchor the unit rotation
        // periodically so roundoff cannot accumulate across the full stage.
        for block in pairs.chunks_exact_mut(width) {
            let (left, right) = block.split_at_mut_checked(half)?;
            for (group, (left, right)) in left
                .chunks_mut(TWIDDLE_ANCHOR)
                .zip(right.chunks_mut(TWIDDLE_ANCHOR))
                .enumerate()
            {
                let turns = (group * TWIDDLE_ANCHOR) as f32 * spacing;
                let (mut sin, mut cos) = if group == 0 {
                    (0.0, 1.0)
                } else {
                    (sine(turns), sine(turns + 0.25))
                };
                for (a, b) in left.iter_mut().zip(right) {
                    let real = b[0] * cos - b[1] * sin;
                    let imag = b[0] * sin + b[1] * cos;
                    *b = [a[0] - real, a[1] - imag];
                    *a = [a[0] + real, a[1] + imag];
                    (sin, cos) = (
                        sin * cos_step + cos * sin_step,
                        cos * cos_step - sin * sin_step,
                    );
                }
            }
        }
        width *= 2;
    }
    Some(())
}

pub(super) fn spectral_pulse(
    patch: &LayerPatch,
    rate: u32,
    waveform: &mut [f32],
    workspace: &mut [f32],
    tables: Option<Lookup<'_>>,
) -> Option<()> {
    let size = (waveform.len() * 2).next_power_of_two().max(4);
    let workspace = workspace.get_mut(..size)?;
    let spectrum = Spectrum::new(&patch.controls, rate, size);
    let (pairs, _) = workspace.as_chunks_mut::<2>();
    let count = pairs.len();
    pack_spectrum(&spectrum, pairs)?;
    inverse_transform(pairs)?;
    let scale = 1.0 / count as f32;
    let mut samples = workspace.iter().copied();
    render_layer(patch.controls[6], rate, waveform, tables, |_| {
        samples.next().unwrap_or(0.0) * scale
    });
    Some(())
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::Model;
    use std::vec;

    #[test]
    fn spectral_pulse_matches_web_ifft() {
        for (controls, length, expected) in [
            (
                [54.0, 345.0, 75.0, 720.0, 38.0, 1.2, 2.0, 0.0, 0.0, 0.0],
                25080,
                [
                    0.00018548, 0.25265227, 0.84220802, 0.70889706, 0.38510622, 0.01175425,
                    0.00000507,
                ],
            ),
            (
                [75.0, 850.0, 100.0, 3800.0, 100.0, 20.0, 2.0, 0.0, 0.0, 0.0],
                48000,
                [
                    -0.00048658,
                    0.32360,
                    0.68277412,
                    -0.62236678,
                    -0.00000989,
                    0.45071790,
                    -0.00081573,
                ],
            ),
        ] {
            let patch = LayerPatch {
                model: Model::SpectralPulse,
                controls,
            };
            let mut waveform = vec![0.0; length];
            let mut workspace = vec![0.0; spectral_workspace_frames(48_000)];
            spectral_pulse(&patch, 48_000, &mut waveform, &mut workspace, None).unwrap();
            for (frame, target) in [1, 37, 237, 1201, 4800, 16183, length - 121]
                .into_iter()
                .zip(expected)
            {
                assert!(
                    (waveform[frame] - target).abs() < 0.0015,
                    "frame {frame}: {} != {target}",
                    waveform[frame]
                );
            }
            assert_eq!(waveform[0], 0.0);
            assert_eq!(waveform[length - 1], 0.0);
        }
    }
}
