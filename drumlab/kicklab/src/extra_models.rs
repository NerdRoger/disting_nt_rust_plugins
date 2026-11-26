use super::{
    decay_rate, exp2, filter_coefficient, lookup, next_noise_bits, positive_pow, render_layer,
    sine, DecayEnvelope, LayerPatch, LayerShape, Lookup, NoiseBand, NOISE_SEED,
};

fn rotation(step: f32) -> (f32, f32) {
    let half_sine = sine(0.5 * step);
    (sine(step), 1.0 - 2.0 * half_sine * half_sine)
}

pub(super) fn resonator(
    layer: &LayerPatch,
    rate: u32,
    waveform: &mut [f32],
    tables: Option<Lookup<'_>>,
) {
    let [tune, decay, depth, sweep_time, drift, impact, width, tone, drive, _] = layer.controls;
    let base_step = tune / rate as f32;
    let sweep_octaves = depth / 12.0;
    let drift_octaves = drift / 12.0;
    let mut sweep_envelope = DecayEnvelope::new(decay_rate(sweep_time, rate));
    let mut drift_envelope = DecayEnvelope::new(decay_rate(90.0, rate));
    let mut moving = depth != 0.0 || drift != 0.0;
    let steady_rotation = rotation(base_step);
    let damping = exp2(decay_rate(decay, rate));
    let tone = filter_coefficient(tone, rate);
    let pulse_frames = width * rate as f32 / 1000.0;
    let pulse_gain = impact / 100.0 * positive_pow((0.5 / width).min(1.0), 0.5);
    let shape = LayerShape::new(drive, rate, waveform.len(), tables);
    let (mut real, mut imaginary, mut filtered) = (0.0, 0.0, 0.0);
    for (frame, sample) in waveform.iter_mut().enumerate() {
        let t = frame as f32;
        let (sin_step, cos_step) = if moving {
            let octaves = sweep_octaves * sweep_envelope.next::<256>(frame)
                + drift_octaves * drift_envelope.next::<256>(frame);
            let multiplier = lookup::pitch_multiplier(tables, octaves);
            moving = multiplier != 1.0;
            rotation(base_step * multiplier)
        } else {
            steady_rotation
        };
        let pulse = if t < pulse_frames {
            sine(t / pulse_frames) * pulse_gain
        } else {
            0.0
        };
        (real, imaginary) = (
            damping * (real * cos_step - imaginary * sin_step),
            damping * (imaginary * cos_step + real * sin_step + pulse),
        );
        filtered += tone * (1.5 * real + 0.32 * pulse - filtered);
        let fade = ((shape.last - frame) as f32 * shape.fade).min(1.0);
        *sample = shape.drive(filtered) * fade;
    }
}

fn advance_phase(phase: &mut f64, step: f32) {
    *phase += f64::from(step);
    *phase -= f64::from(*phase as u32);
}

fn signed_pitch_multiplier(tables: Option<Lookup<'_>>, octaves: f32) -> f32 {
    let multiplier = lookup::pitch_multiplier(tables, octaves.abs());
    if octaves < 0.0 {
        multiplier.recip()
    } else {
        multiplier
    }
}

pub(super) fn two_operator_fm(
    layer: &LayerPatch,
    rate: u32,
    waveform: &mut [f32],
    tables: Option<Lookup<'_>>,
) {
    let [tune, decay, depth, sweep_time, amount, ratio, fm_time, mod_sweep, drive, _] =
        layer.controls;
    let carrier_step = tune / rate as f32;
    let modulator_step = carrier_step * ratio;
    let carrier_octaves = depth / 12.0;
    let modulator_octaves = mod_sweep / 12.0;
    let index = amount / core::f32::consts::TAU;
    let sweep_decay = decay_rate(sweep_time, rate);
    let fm_decay = decay_rate(fm_time, rate);
    let mut sweeping = carrier_octaves != 0.0;
    let mut modulator_sweeping = modulator_octaves != 0.0;
    let mut modulating = amount != 0.0;
    let mut body = DecayEnvelope::new(decay_rate(decay, rate));
    let (mut carrier, mut modulator) = (0.0, 0.0);
    render_layer(drive, rate, waveform, tables, |frame| {
        let phase_offset = if modulating {
            let fm_level = exp2(frame as f32 * fm_decay);
            let current_index = index * fm_level;
            // Below this point the phase change is smaller than the f32
            // sine input can represent around its two-turn offset.
            if current_index <= 1.0e-8 {
                modulating = false;
                0.0
            } else {
                let offset = current_index * sine(modulator as f32);
                let multiplier = if modulator_sweeping {
                    let multiplier = signed_pitch_multiplier(tables, modulator_octaves * fm_level);
                    modulator_sweeping = multiplier != 1.0;
                    multiplier
                } else {
                    1.0
                };
                advance_phase(&mut modulator, modulator_step * multiplier);
                offset
            }
        } else {
            0.0
        };
        // The signed phase offset can exceed one turn at maximum FM.
        let sample = sine(2.0 + carrier as f32 + phase_offset);
        let multiplier = if sweeping {
            let multiplier = lookup::pitch_multiplier(
                tables,
                carrier_octaves * exp2(frame as f32 * sweep_decay),
            );
            sweeping = multiplier != 1.0;
            multiplier
        } else {
            1.0
        };
        advance_phase(&mut carrier, carrier_step * multiplier);
        sample * body.next::<256>(frame)
    });
}

#[derive(Clone, Copy)]
struct NoiseCoefficients {
    g: f32,
    response: f32,
    compensation: f32,
}

impl NoiseCoefficients {
    fn new(frequency: f32, focus: f32, rate: f32) -> Self {
        let half_turns = 0.5 * frequency / rate;
        let g = sine(half_turns) / sine(half_turns + 0.25);
        Self {
            g,
            response: (1.0 + g * (g + focus.recip())).recip(),
            compensation: (0.55 * positive_pow(rate / (frequency * focus), 0.5)).min(12.0),
        }
    }
}

pub(super) fn noise_sweep(
    layer: &LayerPatch,
    rate: u32,
    waveform: &mut [f32],
    tables: Option<Lookup<'_>>,
) {
    let [tune, decay, depth, sweep_time, focus, q_sweep, grain, drive, ..] = layer.controls;
    let octaves = depth / 12.0;
    let mut sweep_envelope = DecayEnvelope::new(decay_rate(sweep_time, rate));
    let mut moving = depth != 0.0 || q_sweep != 0.0;
    let steady = NoiseCoefficients::new(tune, focus, rate as f32);
    let mut body = DecayEnvelope::new(decay_rate(decay, rate));
    let texture = 0.85 * grain / 100.0;
    let texture_step = 73.0 / rate as f32;
    let (mut band_state, mut low_state) = (0.0, 0.0);
    let mut seed = NOISE_SEED;

    render_layer(drive, rate, waveform, tables, |frame| {
        let time = frame as f32;
        let coefficients = if moving {
            let sweep = sweep_envelope.next::<256>(frame);
            let frequency = tune * lookup::pitch_multiplier(tables, octaves * sweep);
            let q = if q_sweep == 0.0 {
                focus
            } else {
                (focus * signed_pitch_multiplier(tables, q_sweep * sweep))
                    .max(0.25)
                    .min(32.0)
            };
            moving = frequency != tune || q != focus;
            if moving {
                NoiseCoefficients::new(frequency, q, rate as f32)
            } else {
                steady
            }
        } else {
            steady
        };
        let noise = next_noise_bits(&mut seed) as f32 / 2_147_483_648.0 - 1.0;
        let band = (band_state + coefficients.g * (noise - low_state)) * coefficients.response;
        let low = low_state + coefficients.g * band;
        band_state = 2.0 * band - band_state;
        low_state = 2.0 * low - low_state;
        let amplitude = if texture == 0.0 {
            1.0
        } else {
            1.0 + texture * sine(time * texture_step)
        };
        band * coefficients.compensation * body.next::<256>(frame) * amplitude
    });
}

fn sine_phase(turns: f64) -> f32 {
    sine((turns - f64::from(turns as u32)) as f32)
}

pub(super) fn ring_mod(
    layer: &LayerPatch,
    rate: u32,
    waveform: &mut [f32],
    tables: Option<Lookup<'_>>,
) {
    let [tune, decay, sweep, time, ratio, level, clang_time, clang_sweep, drive, _] =
        layer.controls;
    let base_step = tune / rate as f32;
    let mut sweep_envelope = DecayEnvelope::new(decay_rate(time, rate));
    let mut clang_envelope = DecayEnvelope::new(decay_rate(clang_time, rate));
    let mut sweeping = sweep != 0.0;
    let mut body = DecayEnvelope::new(decay_rate(decay, rate));
    let sum_ratio = f64::from(2.0 * ratio + 1.0);
    let mut phase = 0.0;
    let mut offset = 0.0;
    render_layer(drive, rate, waveform, tables, |frame| {
        let difference = sine_phase(phase + 0.25);
        let multiplier = if sweeping {
            let value =
                lookup::pitch_multiplier(tables, sweep / 12.0 * sweep_envelope.next::<256>(frame));
            sweeping = value != 1.0;
            value
        } else {
            1.0
        };
        let step = f64::from(base_step * multiplier);
        let clang = if level != 0.0 {
            let envelope = clang_envelope.next::<256>(frame);
            // 2*sin(a)*sin(b) = cos(a-b)-cos(a+b). Only the sum remains
            // after subtracting the difference tone.
            let s = -sine_phase(phase * sum_ratio + offset + 0.25);
            if clang_sweep != 0.0 {
                let bend = signed_pitch_multiplier(tables, clang_sweep / 12.0 * envelope);
                offset += step * sum_ratio * (f64::from(bend) - 1.0);
            }
            s * level / 100.0 * envelope
        } else {
            0.0
        };
        phase += step;
        (difference + clang) * body.next::<256>(frame)
    });
}

fn fold_sample(value: f32) -> f32 {
    let turns = (value + 1.0) * 0.25;
    let fraction = turns - turns as i32 as f32;
    let fraction = if fraction < 0.0 {
        fraction + 1.0
    } else {
        fraction
    };
    1.0 - (4.0 * fraction - 2.0).abs()
}

pub(super) fn wavefold(
    layer: &LayerPatch,
    rate: u32,
    waveform: &mut [f32],
    tables: Option<Lookup<'_>>,
) {
    const OVERSAMPLE: u32 = 4;
    let [tune, decay, sweep, time, folds, fold_time, bias, clean, drive, _] = layer.controls;
    let base_step = tune / (rate * OVERSAMPLE) as f32;
    let lowpass = filter_coefficient(14_000.0, rate * OVERSAMPLE);
    let dc_tracking = filter_coefficient(16.0, rate);
    let mut sweep_envelope = DecayEnvelope::new(decay_rate(time, rate));
    let mut fold_envelope = DecayEnvelope::new(decay_rate(fold_time, rate));
    let mut sweeping = sweep != 0.0;
    let mut folding = folds != 0.0;
    let clean_level = clean / 100.0;
    let mut body = DecayEnvelope::new(decay_rate(decay, rate));
    let (mut phase, mut filtered, mut dc) = (0.0, 0.0, 0.0);
    render_layer(drive, rate, waveform, tables, |frame| {
        let multiplier = if sweeping {
            let value =
                lookup::pitch_multiplier(tables, sweep / 12.0 * sweep_envelope.next::<256>(frame));
            sweeping = value != 1.0;
            value
        } else {
            1.0
        };
        let step = f64::from(base_step * multiplier);
        if clean_level == 1.0 {
            let clean = sine_phase(phase + step * f64::from(OVERSAMPLE - 1));
            phase += step * f64::from(OVERSAMPLE);
            return clean * body.next::<256>(frame);
        }
        let amount = if folding {
            let value = 1.0 + folds * fold_envelope.next::<256>(frame);
            folding = value != 1.0;
            value
        } else {
            1.0
        };
        let offset = bias / 100.0 * (1.0 - amount.recip());
        let mut clean = 0.0;
        for _ in 0..OVERSAMPLE {
            clean = sine_phase(phase);
            let folded = if folding {
                fold_sample((clean + offset) * amount)
            } else {
                clean
            };
            filtered += lowpass * (folded - filtered);
            phase += step;
        }
        dc += dc_tracking * (filtered - dc);
        let wet = filtered - dc;
        (wet + (clean - wet) * clean_level) * body.next::<256>(frame)
    });
}

fn loop_atan(x: f32) -> f32 {
    // The loop's minimum cutoff and maximum tune bound x below 0.5.
    let squared = x * x;
    x * (1.0
        - squared
            * (1.0 / 3.0
                - squared
                    * (1.0 / 5.0
                        - squared
                            * (1.0 / 7.0
                                - squared
                                    * (1.0 / 9.0 - squared * (1.0 / 11.0 - squared / 13.0))))))
}

pub(super) fn delay_drum(
    layer: &LayerPatch,
    rate: u32,
    waveform: &mut [f32],
    layer_cache: &mut [f32],
    tables: Option<Lookup<'_>>,
) -> Option<()> {
    let [tune, decay, cutoff, strike_mix, strike_width, strike_tone, drive, ..] = layer.controls;
    let period = rate as f32 / tune;
    let phase_step = tune / rate as f32;
    let tone = filter_coefficient(cutoff, rate);
    let pole = 1.0 - tone;
    let (sin_step, cos_step) = rotation(phase_step);
    let real = 1.0 - pole * cos_step;
    let imaginary = pole * sin_step;
    let delay = period - loop_atan(imaginary / real) / (core::f32::consts::TAU * phase_step);
    let magnitude = tone * positive_pow(real * real + imaginary * imaginary, -0.5);
    let feedback = (exp2(decay_rate(decay, rate) * period) / magnitude).min(0.9999);
    let whole = period as usize;
    let size = whole + usize::from((whole as f32) < period) + 1;
    let buffer = layer_cache.get_mut(..size)?;
    buffer.fill(0.0);
    let mut noise = NoiseBand::new(80.0, strike_tone, rate);
    let strike_frames = strike_width * rate as f32 / 1000.0;
    let strike_adjustment = strike_mix / 100.0 - 1.0;
    let tracking = filter_coefficient(14.0, rate);
    let shape = LayerShape::new(drive, rate, waveform.len(), tables);
    let delay_floor = delay as usize;
    let delay_ceil = delay_floor + usize::from((delay_floor as f32) < delay);
    let fraction = delay_ceil as f32 - delay;
    let mut read = size - delay_ceil;
    let (mut write, mut filtered, mut dc) = (0, 0.0, 0.0);
    for (frame, output) in waveform.iter_mut().enumerate() {
        let next = if read + 1 == size { 0 } else { read + 1 };
        let first = *buffer.get(read)?;
        let delayed = first + (*buffer.get(next)? - first) * fraction;
        read = next;
        filtered += tone * (delayed - filtered);
        let t = frame as f32;
        let strike = if t < strike_frames {
            noise.next() * sine(0.5 * t / strike_frames) * 0.85
        } else {
            0.0
        };
        let sample = filtered * feedback + strike;
        *buffer.get_mut(write)? = sample;
        write = if write + 1 == size { 0 } else { write + 1 };
        let raw = sample + strike * strike_adjustment;
        dc += tracking * (raw - dc);
        let fade = ((shape.last - frame) as f32 * shape.fade).min(1.0);
        *output = shape.drive(raw - dc) * fade;
    }
    Some(())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;

    fn render_delay_fixture(
        layer: &LayerPatch,
        rate: u32,
        waveform: &mut [f32],
        tables: Option<Lookup<'_>>,
    ) {
        let mut scratch = vec![0.0; rate as usize / 25 + 2];
        delay_drum(layer, rate, waveform, &mut scratch, tables).unwrap();
    }

    #[test]
    fn delay_drum_matches_web_samples() {
        check_web_samples(
            crate::Model::DelayDrum,
            [64.0, 520.0, 1700.0, 100.0, 5.7, 2600.0, 5.0, 0.0, 0.0, 0.0],
            37680,
            render_delay_fixture,
            &[
                (1, 0.006559371),
                (37, 0.31265035),
                (237, -0.2221296),
                (1201, 0.014622292),
                (4800, 0.010430664),
                (16183, 0.0010523015),
                (37559, 0.000040873308),
            ],
        );
        check_web_samples(
            crate::Model::DelayDrum,
            [25.0, 1000.0, 250.0, 0.0, 20.0, 8500.0, 5.0, 0.0, 0.0, 0.0],
            48000,
            render_delay_fixture,
            &[
                (1, 0.0),
                (37, 0.0),
                (237, 0.0),
                (1201, 0.0),
                (4800, -0.008305794),
                (16183, -0.0048864726),
                (47879, -0.000012718077),
            ],
        );
    }

    #[test]
    fn wavefold_matches_web_samples() {
        check_web_samples(
            crate::Model::Wavefold,
            [49.0, 430.0, 16.0, 28.0, 4.2, 158.0, 26.0, 0.0, 1.0, 0.0],
            31200,
            wavefold,
            &[
                (1, 0.023468584),
                (37, -0.15441398),
                (237, -0.09630993),
                (1201, -0.6485241),
                (4800, 0.2114741),
                (16183, -0.011771432),
                (31079, 0.00035029485),
            ],
        );
        check_web_samples(
            crate::Model::Wavefold,
            [
                75.0, 1000.0, 60.0, 200.0, 10.0, 650.0, -80.0, 100.0, 1.0, 0.0,
            ],
            48000,
            wavefold,
            &[
                (1, 0.01591799),
                (37, -0.84552443),
                (237, -0.6087348),
                (1201, -0.7815895),
                (4800, -0.564664),
                (16183, -0.20218143),
                (47879, 0.005103949),
            ],
        );
    }

    #[test]
    fn ring_mod_matches_web_samples() {
        check_web_samples(
            crate::Model::RingMod,
            [52.0, 420.0, 17.0, 32.0, 3.5, 65.0, 102.0, 0.0, 3.0, 0.0],
            30480,
            ring_mod,
            &[
                (1, 0.0132622104),
                (37, 0.61322494),
                (237, -0.9883295),
                (1201, -0.14178792),
                (4800, -0.2059915),
                (16183, 0.030932835),
                (30359, -0.00037169476),
            ],
        );
        check_web_samples(
            crate::Model::RingMod,
            [
                75.0, 1000.0, 60.0, 200.0, 9.0, 100.0, 600.0, -24.0, 3.0, 0.0,
            ],
            48000,
            ring_mod,
            &[
                (1, 0.032385852),
                (37, 1.0376002),
                (237, -0.3375842),
                (1201, 0.23120496),
                (4800, -0.6275869),
                (16183, -0.07529909),
                (47879, 0.0011345178),
            ],
        );
    }

    #[test]
    fn noise_sweep_matches_web_samples() {
        check_web_samples(
            crate::Model::NoiseSweep,
            [58.0, 260.0, 41.0, 27.0, 4.1, 0.0, 12.0, 4.0, 0.0, 0.0],
            18960,
            noise_sweep,
            &[
                (1, 0.0076195975),
                (37, 0.11461962),
                (237, 0.43377867),
                (1201, 0.32874545),
                (4800, -0.08660231),
                (16183, -0.003284285),
                (18839, 0.0009937668),
            ],
        );
        check_web_samples(
            crate::Model::NoiseSweep,
            [75.0, 1000.0, 60.0, 200.0, 0.5, -3.0, 100.0, 4.0, 0.0, 0.0],
            48000,
            noise_sweep,
            &[
                (1, 0.022827115),
                (37, -0.03661971),
                (237, -0.90624547),
                (1201, 0.04359146),
                (4800, 0.5116278),
                (16183, -0.09722912),
                (37501, -0.006220961),
                (47879, -0.00048038198),
            ],
        );
    }

    fn check_web_samples(
        model: crate::Model,
        controls: [f32; 10],
        length: usize,
        renderer: fn(&LayerPatch, u32, &mut [f32], Option<Lookup<'_>>),
        points: &[(usize, f32)],
    ) {
        let layer = LayerPatch { model, controls };
        let mut waveform = vec![0.0; length];
        let mut memory = [0.0; lookup::BYTES as usize / 4];
        renderer(
            &layer,
            48_000,
            &mut waveform,
            Some(lookup::test_tables(&mut memory)),
        );
        for &(frame, expected) in points {
            let sample = waveform[frame];
            assert!(
                (sample - expected).abs() < 0.0015,
                "{frame}: {sample} vs {expected}"
            );
        }
        assert_eq!(waveform[0], 0.0);
        assert_eq!(waveform[length - 1], 0.0);
        assert!(waveform.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn two_operator_fm_supports_downward_modulator_sweeps() {
        check_web_samples(
            crate::Model::TwoOperatorFm,
            [49.0, 1000.0, 24.0, 22.0, 9.0, 16.0, 400.0, -24.0, 3.0, 0.0],
            48000,
            two_operator_fm,
            &[
                (37, 0.90696526),
                (237, -0.78922206),
                (1201, 0.7120785),
                (4800, 0.15825266),
                (16183, -0.025310317),
                (37501, -0.033958048),
            ],
        );
    }

    #[test]
    fn two_operator_fm_matches_web_samples() {
        check_web_samples(
            crate::Model::TwoOperatorFm,
            [49.0, 420.0, 24.0, 22.0, 3.2, 3.45, 78.0, 0.0, 3.0, 0.0],
            30480,
            two_operator_fm,
            &[
                (1, 0.0035719082),
                (37, 0.06291049),
                (237, 0.89840406),
                (1201, -0.71784365),
                (4800, 0.2718682),
                (16183, 0.00092692603),
                (30359, 0.00009080979),
            ],
        );
        check_web_samples(
            crate::Model::TwoOperatorFm,
            [75.0, 1000.0, 60.0, 200.0, 9.0, 16.0, 400.0, 36.0, 3.0, 0.0],
            48000,
            two_operator_fm,
            &[
                (1, 0.01950266),
                (37, 0.16899173),
                (237, -0.1435741),
                (1201, -0.86334187),
                (4800, -0.4651699),
                (16183, -0.25470445),
                (37501, 0.0011610376),
                (47879, 0.0062072924),
            ],
        );
    }

    #[test]
    fn resonator_deep_sweep_matches_web_samples() {
        check_web_samples(
            crate::Model::Resonator,
            [75.0, 1000.0, 60.0, 200.0, 6.0, 65.0, 3.0, 2000.0, 1.0, 0.0],
            48000,
            resonator,
            &[
                (1, 0.000873972),
                (37, -0.80493033),
                (237, 0.024084123),
                (1201, -0.06255812),
                (4800, 0.051649854),
                (16183, 0.01423785),
                (37501, -0.0035719147),
                (47879, -0.00022771656),
            ],
        );
    }

    #[test]
    fn resonator_matches_default_web_samples() {
        let layer = LayerPatch {
            model: crate::Model::Resonator,
            controls: [53.0, 820.0, 13.0, 12.0, 1.5, 65.0, 0.5, 2000.0, 1.0, 0.0],
        };
        let mut waveform = vec![0.0; 48_000];
        let mut memory = [0.0; lookup::BYTES as usize / 4];
        resonator(
            &layer,
            48_000,
            &mut waveform,
            Some(lookup::test_tables(&mut memory)),
        );
        for (frame, expected) in [
            (1, 0.012702339),
            (37, -1.1797932),
            (237, 0.8556812),
            (1201, 1.0927625),
            (4800, 0.74224246),
            (16183, -0.18679312),
            (47879, -0.0023109603),
        ] {
            let sample = waveform[frame];
            assert!(
                (sample - expected).abs() < 0.0015,
                "{frame}: {sample} vs {expected}"
            );
        }
    }
}
