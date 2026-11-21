use super::{exp2, positive_pow, render_layer, sine, LayerPatch, Lookup, LOG2_E, TAU};
use core::f32::consts::LOG2_10;

const SNAP_DELAY_SECONDS: f32 = 0.008;
// Both envelopes are quiet below this power (about -157 dB).
const MIN_POWER: f32 = -26.0;

struct Snap {
    center: f32,
    envelope: f32,
    chirp: f32,
    real_gain: f32,
    quadrature_gain: f32,
}

impl Snap {
    fn new(level: f32, center: f32, bandwidth: f32, dispersion_ms: f32) -> Self {
        let width = center * bandwidth * 0.01;
        let angular_width = TAU * width;
        let dispersion = angular_width * dispersion_ms * 0.001;
        // Inverting the Gaussian band with quadratic phase gives a Gaussian
        // envelope and a quadratic time-domain phase. Evaluate the full band
        // analytically, including the tails beyond the old FFT bin limits.
        let inverse_norm = positive_pow(1.0 + dispersion * dispersion, -0.5);
        let curvature = 0.5 * (angular_width * inverse_norm) * (angular_width * inverse_norm);
        let gain = 0.007 * level;
        // (1 + i*dispersion)^(-1/2) sets the phase and amplitude at the center
        // of the snap. Use its conjugate's imaginary part to weight sine.
        let in_phase = positive_pow(0.5 * inverse_norm * (1.0 + inverse_norm), 0.5);
        let quadrature = 0.5 * (dispersion * inverse_norm) * inverse_norm / in_phase;
        Self {
            center,
            envelope: -curvature * LOG2_E,
            chirp: curvature * dispersion / TAU,
            real_gain: gain * in_phase,
            quadrature_gain: gain * quadrature,
        }
    }

    fn sample(&self, seconds: f32) -> f32 {
        let time = seconds - SNAP_DELAY_SECONDS;
        let power = self.envelope * time * time;
        if power < MIN_POWER {
            return 0.0;
        }
        let turns = time * (self.center + self.chirp * time);
        // The oscillator helper expects nonnegative turns. Keep enough phase
        // precision before the 8 ms center and for the late dispersed snap.
        let phase = turns - turns as i32 as f32 + 1.0;
        let carrier = self.real_gain * sine(phase + 0.25) + self.quadrature_gain * sine(phase);
        carrier * exp2(power)
    }
}

pub(super) fn spectral_pulse(
    patch: &LayerPatch,
    rate: u32,
    waveform: &mut [f32],
    tables: Option<Lookup<'_>>,
) {
    let [tune, decay_ms, snap_level, center, bandwidth, dispersion_ms, drive, ..] = patch.controls;
    let snap = (snap_level > 0.0).then(|| Snap::new(snap_level, center, bandwidth, dispersion_ms));
    let seconds_per_frame = 1.0 / rate as f32;
    let decay_seconds = 0.001 * decay_ms;
    let bass_envelope = -2.0 * LOG2_10 / (decay_seconds * decay_seconds);
    render_layer(drive, rate, waveform, tables, |frame| {
        let seconds = frame as f32 * seconds_per_frame;
        let power = bass_envelope * seconds * seconds;
        let bass = if power >= MIN_POWER {
            0.9 * exp2(power) * sine(tune * seconds)
        } else {
            0.0
        };
        bass + snap.as_ref().map_or(0.0, |snap| snap.sample(seconds))
    });
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::Model;
    use std::vec;

    #[test]
    fn spectral_pulse_preserves_web_reference_at_default_and_high_dispersion() {
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
            spectral_pulse(&patch, 48_000, &mut waveform, None);
            for (frame, target) in [1, 37, 237, 1201, 4800, 16183, length - 121]
                .into_iter()
                .zip(expected)
            {
                assert!(
                    (waveform[frame] - target).abs() < 0.0025,
                    "frame {frame}: {} != {target}",
                    waveform[frame]
                );
            }
            assert_eq!(waveform[0], 0.0);
            assert_eq!(waveform[length - 1], 0.0);
        }
    }
}
