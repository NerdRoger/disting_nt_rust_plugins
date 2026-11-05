use super::*;

#[test]
fn rms_control_keeps_correlated_and_unrelated_chords_near_single_voice_level() {
    for correlated in [false, true] {
        for voices in [1, 2, 5, 10] {
            let mut control = LevelControl::new(48000);
            let mut energy = 0.0;
            let mut reference_energy = 0.0;
            for block in 0..12000 {
                let mut mix = [0.0; CHUNK];
                let mut voice_power = 0.0;
                for voice in 0..voices {
                    let frequency = if correlated {
                        220.0
                    } else {
                        220.0 + voice as f32 * 47.0
                    };
                    for (i, sum) in mix.iter_mut().enumerate() {
                        let phase = core::f32::consts::TAU * frequency * (block * CHUNK + i) as f32
                            / 48000.0;
                        let sample = phase.sin();
                        *sum += sample;
                        voice_power += sample * sample;
                    }
                }
                // Generous headroom separates the RMS behavior from limiting.
                control.process(&mut mix, voice_power, voices as f32, 10.0);
                if block > 9000 {
                    energy += mix.iter().map(|x| x * x).sum::<f32>();
                    reference_energy += voice_power / voices as f32;
                }
            }
            let ratio = (energy / reference_energy).sqrt();
            assert!(
                (ratio - 1.0).abs() < 0.07,
                "voices {voices}, correlated {correlated}: {ratio}"
            );
        }
    }
}

#[test]
fn peak_protection_bounds_sudden_chords_and_does_not_release_each_cycle() {
    let mut control = LevelControl::new(48000);
    for block in 0..3000 {
        let mut mix = [0.0; CHUNK];
        let mut voice_power = 0.0;
        for (i, sample) in mix.iter_mut().enumerate() {
            let value =
                (core::f32::consts::TAU * 55.0 * (block * CHUNK + i) as f32 / 48000.0).sin();
            *sample = 10.0 * value;
            voice_power += 10.0 * value * value;
        }
        control.process(&mut mix, voice_power, 10.0, 1.0);
        assert!(mix.iter().all(|x| x.abs() <= 1.000001));
        if block > 1000 {
            assert!(
                control.peak_gain < 0.11,
                "limiter must hold through troughs"
            );
        }
    }
}

#[test]
fn silence_and_release_are_not_boosted_and_additive_contribution_stays_separate() {
    let mut control = LevelControl::new(48000);
    let mut loud = [10.0; CHUNK];
    control.process(&mut loud, CHUNK as f32 * 10.0, 10.0, 1.0);
    let gain = control.gain;
    for _ in 0..5000 {
        let mut silent = [0.0; CHUNK];
        control.process(&mut silent, 0.0, 1.0, 1.0);
        assert_eq!(silent, [0.0; CHUNK]);
    }
    assert!((control.gain - gain).abs() < 0.0001);
    let mut tail = [0.00001; CHUNK];
    control.process(&mut tail, CHUNK as f32 * 0.00001 * 0.00001, 1.0, 1.0);
    assert!(tail.iter().all(|x| *x <= 0.00001));
    control.process(&mut tail, 0.0, 0.0, 1.0);
    assert_eq!(tail, [0.0; CHUNK]);
    assert_eq!(control.gain, 1.0);
}

#[test]
fn square_root_matches_native_math() {
    for i in 0..=1000 {
        let value = i as f32 / 1000.0;
        assert!((sqrt(value) - value.sqrt()).abs() < 0.000001);
    }
}

#[test]
fn adding_a_second_note_at_half_level_keeps_rms_and_does_not_engage_peak_protection() {
    let mut control = LevelControl::new(48000);
    let mut mono_energy = 0.0;
    let mut chord_energy = 0.0;
    for block in 0..12000 {
        let mut mix = [0.0; CHUNK];
        let mut voice_power = 0.0;
        let mut gate_power = 0.0;
        for (i, sample) in mix.iter_mut().enumerate() {
            let frame = block * CHUNK + i;
            let first = 2.5 * (core::f32::consts::TAU * 220.0 * frame as f32 / 48000.0).sin();
            let gate = if frame < 96000 {
                0.0
            } else {
                ((frame - 96000) as f32 / 240.0).min(1.0)
            };
            let second =
                2.5 * gate * (core::f32::consts::TAU * 330.0 * frame as f32 / 48000.0).sin();
            *sample = first + second;
            voice_power += first * first + second * second;
            gate_power += 1.0 + gate * gate;
        }
        control.process(
            &mut mix,
            voice_power,
            gate_power / CHUNK as f32,
            PEAK_CEILING,
        );
        assert_eq!(
            control.peak_gain, 1.0,
            "normal two-note chord must have peak headroom"
        );
        if (3000..6000).contains(&block) {
            mono_energy += mix.iter().map(|v| v * v).sum::<f32>();
        }
        if (9000..12000).contains(&block) {
            chord_energy += mix.iter().map(|v| v * v).sum::<f32>();
        }
    }
    let ratio = (chord_energy / mono_energy).sqrt();
    assert!((0.94..1.06).contains(&ratio), "chord/mono RMS: {ratio}");
}

#[test]
fn barely_audible_new_note_does_not_halve_the_loudness_reference() {
    let mut control = LevelControl::new(48000);
    for _ in 0..4000 {
        control.process(&mut [1.0; CHUNK], CHUNK as f32, 1.0, PEAK_CEILING);
    }
    let gate = 0.01;
    let mut mix = [1.0 + gate; CHUNK];
    control.process(
        &mut mix,
        CHUNK as f32 * (1.0 + gate * gate),
        1.0 + gate * gate,
        PEAK_CEILING,
    );
    assert!((control.reference_power - 1.0).abs() < 0.0001);
    assert!(mix.iter().all(|&v| v > 1.0));
}

#[test]
fn mode_transition_is_continuous_retriggerable_and_finishes_after_twenty_ms() {
    for rate in [32000, 48000, 96000] {
        let mut transition = ModeTransition::new(rate);
        let mut previous = [2.0];
        transition.apply(&mut previous);
        transition.start();
        let mut elapsed = 0;
        let mut last = previous[0];
        while elapsed < rate / 50 {
            let mut sample = [-2.0];
            transition.apply(&mut sample);
            if elapsed == 0 {
                assert_eq!(sample[0], 2.0);
            }
            assert!((sample[0] - last).abs() <= 4.0 / (rate / 50) as f32 + 0.00001);
            last = sample[0];
            elapsed += 1;
        }
        let mut next = [-2.0];
        transition.apply(&mut next);
        assert_eq!(next[0], -2.0);
        transition.start();
        let mut rising = [3.0; 24];
        transition.apply(&mut rising);
        let last = rising[23];
        transition.start();
        let mut changed_again = [-3.0];
        transition.apply(&mut changed_again);
        assert_eq!(changed_again[0], last);
        transition.silence();
        transition.start();
        changed_again[0] = 1.25;
        transition.apply(&mut changed_again);
        assert_eq!(changed_again[0], 1.25); // No transition before first render.
    }
}

#[test]
fn mode_restart_acquires_normalized_gain_before_fading_in_coherent_cv_voices() {
    let mut control = LevelControl::new(48000);
    control.restart_for_mode_change();
    let mut silent = [0.0; CHUNK];
    control.process(&mut silent, 0.0, 10.0, PEAK_CEILING);
    assert!(control.prime_gain);
    let mut mix = [10.0; CHUNK];
    control.process(&mut mix, 10.0 * CHUNK as f32, 10.0, PEAK_CEILING);
    assert!(!control.prime_gain);
    assert!(mix.iter().all(|v| (*v - 1.0).abs() < 0.000001));
    assert!((control.gain - 0.1).abs() < 0.000001);
}

#[test]
fn decibel_level_has_default_headroom_true_mute_and_correct_amplitude_ratios() {
    let mut level = Level::new();
    for db in [LEVEL_DEFAULT, 0, -6, -12, -59, LEVEL_MIN, -100, 20, -6] {
        let actual = level.amplitude(db);
        let expected = if db <= LEVEL_MIN {
            0.0
        } else {
            10.0f32.powf(db.min(0) as f32 / 20.0)
        };
        assert!((actual - expected).abs() < 0.000001);
        assert_eq!(level.amplitude(db), actual); // Unchanged settings reuse the cached gain.
    }
}

#[test]
fn stereo_level_control_links_gain_and_peak_protection_without_moving_the_image() {
    let mut control = LevelControl::new(48000);
    for block in 0..4000 {
        let mut left = [0.0; CHUNK];
        let mut right = [0.0; CHUNK];
        let mut power = 0.0;
        for (i, (l, r)) in left.iter_mut().zip(&mut right).enumerate() {
            *l = 20.0
                * (core::f32::consts::TAU * 110.0 * (block * CHUNK + i) as f32 / 48000.0).sin();
            *r = *l * 0.3;
            power += *l * *l + *r * *r;
        }
        control.process_stereo(&mut left, &mut right, power, 10.0, PEAK_CEILING);
        for (&l, &r) in left.iter().zip(&right) {
            assert!(l.abs() <= PEAK_CEILING + 0.000001);
            assert!((r - l * 0.3).abs() < 0.000001);
        }
    }
}

#[test]
fn fused_detector_and_stereo_gain_match_original_controller_bit_for_bit() {
    for stereo in [false, true] {
        for frames in [0, 1, 8, 16, 24, 64] {
            let mut actual = LevelControl::new(48000);
            let mut expected = actual;
            for block in 0..1000 {
                if block % 137 == 0 {
                    actual.restart_for_mode_change();
                    expected.restart_for_mode_change();
                }
                let mut left = [0.0; 64];
                let mut right = [0.0; 64];
                let mut voice_power = 0.0;
                for (i, (l, r)) in left.iter_mut().zip(&mut right).enumerate().take(frames) {
                    let amplitude = if block % 19 == 0 {
                        0.0
                    } else if block % 11 == 0 {
                        20.0
                    } else {
                        2.5
                    };
                    *l = amplitude * ((i * 73 + block * 31) % 257) as f32 / 128.0 - amplitude;
                    *r = *l * 0.37;
                    voice_power += *l * *l + *r * *r;
                }
                let mut el = left;
                let mut er = right;
                let weight = if block % 29 == 0 {
                    0.0
                } else {
                    1.0 + (block % 10) as f32
                };
                let ceiling = if block % 43 == 0 { 0.0 } else { 5.0 };
                if stereo {
                    actual.process_channels::<true>(
                        &mut left[..frames],
                        &mut right[..frames],
                        voice_power,
                        weight,
                        ceiling,
                    );
                    expected.process_channels_reference::<true>(
                        &mut el[..frames],
                        &mut er[..frames],
                        voice_power,
                        weight,
                        ceiling,
                    );
                } else {
                    actual.process_channels::<false>(
                        &mut left[..frames],
                        &mut [],
                        voice_power,
                        weight,
                        ceiling,
                    );
                    expected.process_channels_reference::<false>(
                        &mut el[..frames],
                        &mut [],
                        voice_power,
                        weight,
                        ceiling,
                    );
                }
                for (a, b) in left.iter().chain(&right).zip(el.iter().chain(&er)) {
                    assert_eq!(
                        a.to_bits(),
                        b.to_bits(),
                        "stereo {stereo}, frames {frames}, block {block}"
                    );
                }
                for (a, b) in [
                    (actual.mix_power, expected.mix_power),
                    (actual.reference_power, expected.reference_power),
                    (actual.gain, expected.gain),
                    (actual.peak_gain, expected.peak_gain),
                ] {
                    assert_eq!(a.to_bits(), b.to_bits());
                }
                assert_eq!(
                    (actual.peak_hold, actual.prime_gain),
                    (expected.peak_hold, expected.prime_gain)
                );
            }
        }
    }
}
