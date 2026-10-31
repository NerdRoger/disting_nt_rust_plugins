use super::*;

fn read_table(engine: &Engine, memory: &Memory, index: usize) -> [Complex; SIZE] {
    let mut table = [Complex::default(); SIZE];
    for (i, sample) in table.iter_mut().enumerate() {
        sample.re = engine.lookup(memory, index, (i as u32) << 24);
    }
    table
}

fn rendered_harmonic_gain(increment: f32, harmonic: usize) -> f32 {
    let (level, blend) = playback_levels(increment);
    if harmonic > cutoff(level) {
        0.0
    } else if level + 1 == LEVELS || harmonic <= cutoff(level + 1) {
        1.0
    } else {
        1.0 - blend
    }
}

fn finish_build(engine: &mut Engine, memory: &mut Memory, wave: Wave, frequency: f32) {
    engine.request(wave);
    for _ in 0..BUILD_STEPS + 8 {
        engine.advance(memory);
        for _ in 0..64 {
            assert!(engine.tick(memory, frequency, 48000.0, 1.0).is_finite());
        }
    }
    assert_eq!(engine.waves[engine.active], wave);
    assert_eq!(engine.fade, 0);
}

#[test]
fn curve_is_monotonic_bounded_and_moves_continuously_through_linear() {
    let mut wave = Wave::new(3);
    wave.boundaries[1] = -100;
    wave.boundaries[2] = 100;
    for curve in -100..=100 {
        wave.curves[1] = curve;
        let mut previous = -1.0;
        for i in 0..100 {
            let value = wave.segment_sample(1, i as f32 / 100.0);
            assert!(value >= previous - 0.00001 && value <= 1.0);
            previous = value;
        }
    }
    wave.curves[1] = 0;
    assert_eq!(wave.segment_sample(1, 0.5), 0.0);
    wave.curves[1] = -1;
    assert!(wave.segment_sample(1, 0.5) > 0.0);
    wave.curves[1] = 1;
    assert!(wave.segment_sample(1, 0.5) < 0.0);
}

#[test]
fn source_boundaries_stay_editable_and_playback_seam_stays_continuous() {
    for n in 2..=MAX_SEGMENTS {
        let mut wave = Wave::new(n);
        wave.boundaries[0] = -37;
        for i in 1..n {
            wave.boundaries[i] = (i * 37 % 201) as i16 - 100;
        }
        wave.curves.fill(99);
        for smoothing in [0, 100] {
            wave.smoothing = smoothing;
            for frequency in [20.0, 2800.0, 12500.0] {
                let mut engine = Engine::new(n, 48000);
                let mut memory = [Block::ZERO; BLOCKS];
                finish_build(&mut engine, &mut memory, wave, frequency);
                assert_eq!(wave.sample(0.0), -0.37);
                assert_eq!(wave.sample(1.0), -0.37);
                assert_eq!(
                    engine.display_segment_sample(&memory, 0, 0.0),
                    engine.display_segment_sample(&memory, n - 1, 1.0)
                );
                for boundary in 1..n {
                    assert_eq!(
                        wave.segment_sample(boundary - 1, 1.0),
                        wave.segment_sample(boundary, 0.0)
                    );
                    assert_eq!(
                        engine.display_segment_sample(&memory, boundary - 1, 1.0),
                        engine.display_segment_sample(&memory, boundary, 0.0)
                    );
                }
            }
        }
    }
}

#[test]
fn smoothing_fft_round_trip() {
    let mut engine = Engine::new(4, 48000);
    let mut memory = [Block::ZERO; BLOCKS];
    finish_build(&mut engine, &mut memory, Wave::new(4), 220.0);
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
fn every_table_removes_harmonics_above_its_limit_after_creative_smoothing() {
    for smoothing in [0, 50, 100] {
        let mut wave = Wave::new(32);
        wave.smoothing = smoothing;
        for i in 0..32 {
            wave.boundaries[i] = (i * 73 % 201) as i16 - 100;
            wave.curves[i] = if i % 2 == 0 { 100 } else { -100 };
        }
        let mut engine = Engine::new(32, 48000);
        let mut memory = [Block::ZERO; BLOCKS];
        finish_build(&mut engine, &mut memory, wave, 2800.0);
        for level in 0..LEVELS {
            let mut spectrum =
                read_table(&engine, &memory, engine.active * TABLES_PER_BANK + level);
            fft(&mut spectrum, &memory[TWIDDLES].data, false);
            let cutoff = cutoff(level);
            let mut pass = 0.0;
            let mut stop = 0.0;
            for (k, value) in spectrum.iter().enumerate() {
                let energy = value.re * value.re + value.im * value.im;
                if k.min(SIZE - k) > cutoff || k == SIZE / 2 {
                    stop += energy;
                } else {
                    pass += energy;
                }
            }
            assert!(pass > 0.001);
            assert!(
                stop < pass * 0.00000001,
                "level {level}: stop {stop}, pass {pass}"
            );
        }
    }
}

#[test]
fn table_selection_is_continuous_and_never_plays_an_unsafe_harmonic() {
    for i in 1..=45000 {
        let increment = i as f32 / 100000.0;
        let (level, blend) = playback_levels(increment);
        assert!(level < LEVELS);
        assert!((0.0..=1.0).contains(&blend));
        assert!(cutoff(level) as f32 * increment <= 0.490001);
        for harmonic in 1..SIZE / 2 {
            let frequency = increment * harmonic as f32;
            let gain = rendered_harmonic_gain(increment, harmonic);
            // All harmonics below 12% of the sample rate stay fully passed,
            // even across the wider gaps in the six-table bank.
            if frequency <= 0.12 {
                assert!(gain > 0.99999);
            }
            if frequency >= 0.49 {
                assert_eq!(gain, 0.0);
            }
        }
    }
    let mut engine = Engine::new(8, 48000);
    let mut memory = [Block::ZERO; BLOCKS];
    let mut wave = Wave::new(8);
    wave.curves.fill(100);
    finish_build(&mut engine, &mut memory, wave, 100.0);
    for level in 0..LEVELS - 1 {
        let threshold = 0.49 / cutoff(level) as f32;
        let mut previous = [0.0; SIZE];
        engine.increment = threshold - 0.0000001;
        (engine.playback_level, engine.playback_blend) = playback_levels(engine.increment);
        for (i, sample) in previous.iter_mut().enumerate() {
            *sample = engine.display_sample(&memory, (i as u32) << 24);
        }
        engine.increment = threshold + 0.0000001;
        (engine.playback_level, engine.playback_blend) = playback_levels(engine.increment);
        for (i, &sample) in previous.iter().enumerate() {
            assert!((engine.display_sample(&memory, (i as u32) << 24) - sample).abs() < 0.001);
        }
    }
}

#[test]
fn dense_tables_preserve_safe_harmonics_and_only_fade_the_top_band() {
    let mut wave = Wave::new(32);
    for i in 0..32 {
        wave.boundaries[i] = (i * 73 % 201) as i16 - 100;
        wave.curves[i] = if i % 2 == 0 { 100 } else { -100 };
    }
    let mut engine = Engine::new(32, 48000);
    let mut memory = [Block::ZERO; BLOCKS];
    finish_build(&mut engine, &mut memory, wave, 220.0);
    let mut source_spectrum = read_table(&engine, &memory, engine.active * TABLES_PER_BANK);
    fft(&mut source_spectrum, &memory[TWIDDLES].data, false);
    for frequency in [220.0, 1800.0, 2793.826, 10000.0] {
        for _ in 0..1000 {
            engine.tick(&memory, frequency, 48000.0, 1.0);
        }
        let mut spectrum = [Complex::default(); SIZE];
        for (i, sample) in spectrum.iter_mut().enumerate() {
            sample.re = engine.display_sample(&memory, (i as u32) << 24);
        }
        fft(&mut spectrum, &memory[TWIDDLES].data, false);
        for harmonic in 1..SIZE / 2 {
            let gain = rendered_harmonic_gain(engine.increment, harmonic);
            let source = source_spectrum[harmonic];
            let actual = spectrum[harmonic];
            assert!(
                ((actual.re - source.re * gain) * (2.0 / SIZE as f32)).abs() < 0.00002,
                "cosine partial {harmonic}, frequency {frequency}"
            );
            assert!(
                ((actual.im - source.im * gain) * (2.0 / SIZE as f32)).abs() < 0.00002,
                "sine partial {harmonic}, frequency {frequency}"
            );
        }
    }
    // The six-table bank uses the eight-harmonic table at 1800 Hz.
    assert_eq!(rendered_harmonic_gain(1800.0 / 48000.0, 8), 1.0);
    assert_eq!(rendered_harmonic_gain(1800.0 / 48000.0, 12), 0.0);
    // Around F7 the eighth harmonic is still largely retained, rather than
    // dropping to a four-harmonic table as the original octave blend did.
    assert_eq!(rendered_harmonic_gain(2793.826 / 48000.0, 8), 1.0);
    assert_eq!(rendered_harmonic_gain(220.0 / 48000.0, 30), 1.0);
    assert_eq!(rendered_harmonic_gain(220.0 / 48000.0, 60), 0.0);
}

#[test]
fn every_specification_has_distinct_nested_limits_and_safe_selection() {
    let mut previous = TableLayout::new(1);
    for tables in 1..=MAX_TABLES {
        let layout = TableLayout::new(tables);
        assert_eq!(layout.count, tables);
        assert_eq!(layout.cutoff(0), 127);
        if tables > 1 {
            assert_eq!(layout.cutoff(tables - 1), 1);
            for i in 1..tables {
                assert!(layout.cutoff(i - 1) > layout.cutoff(i));
            }
            for i in 0..previous.count {
                assert!((0..tables).any(|j| previous.cutoff(i) == layout.cutoff(j)));
            }
        }
        for i in 1..=4500 {
            let increment = i as f32 / 10000.0;
            let (level, blend) = layout.playback_levels(increment);
            if tables == 1 {
                assert_eq!((level, blend), (0, 0.0));
            } else {
                assert!(layout.cutoff(level) as f32 * increment <= 0.490001);
                assert!((0.0..=1.0).contains(&blend));
            }
        }
        previous = layout;
    }
    let six = TableLayout::new(6);
    assert_eq!(&six.cutoffs[..6], &[127, 32, 8, 4, 2, 1]);
}

#[test]
fn all_table_counts_build_complete_banks_and_single_table_bypasses_filtering() {
    for tables in 1..=MAX_TABLES {
        let mut storage = [Block::ZERO; blocks(MAX_TABLES)];
        let memory = &mut storage[..blocks(tables)];
        let mut engine = Engine::with_tables(32, 48000, tables);
        let mut wave = Wave::new(32);
        for i in 0..32 {
            wave.boundaries[i] = if i % 2 == 0 { -75 } else { 100 };
            wave.curves[i] = 90;
        }
        for smoothing in [0, 100] {
            wave.smoothing = smoothing;
            engine.request(wave);
            for _ in 0..32 {
                engine.advance(memory);
                for _ in 0..64 {
                    assert!(engine.tick(memory, 2800.0, 48000.0, 1.0).is_finite());
                }
            }
            assert_eq!(engine.waves[engine.active], wave);
            assert_eq!(engine.fade, 0);
            let mut actual = [Complex::default(); SIZE];
            for (i, sample) in actual.iter_mut().enumerate() {
                sample.re = engine.display_sample(memory, (i as u32) << 24);
                if tables == 1 {
                    let segment_position = i as f32 * wave.count as f32 / SIZE as f32;
                    let segment = segment_position as usize;
                    let defined = engine.display_segment_sample(
                        memory,
                        segment,
                        segment_position - segment as f32,
                    );
                    assert!((sample.re - defined).abs() < 0.00001);
                }
            }
            if tables > 1 {
                fft(&mut actual, &memory[tables + 3].data, false);
                let cap = engine.layout.cutoff(engine.playback_level);
                let energy: f32 = actual.iter().map(|v| v.re * v.re + v.im * v.im).sum();
                let stop: f32 = actual
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| (*k).min(SIZE - *k) > cap)
                    .map(|(_, v)| v.re * v.re + v.im * v.im)
                    .sum();
                assert!(stop < energy * 0.00000001);
            }
        }
    }
}

#[test]
fn high_pitch_folded_tone_is_suppressed_relative_to_direct_playback() {
    // Coherent capture near F7: the 16th harmonic folds to FFT bin 272.
    // Analyze that actual audio spur, rather than only the stored table bins.
    const FRAMES: usize = 4096;
    const FUNDAMENTAL: usize = 239;
    const ALIAS: usize = FRAMES - 16 * FUNDAMENTAL;
    let frequency = FUNDAMENTAL as f32 * 48000.0 / FRAMES as f32;
    let mut wave = Wave::new(32);
    for i in 0..32 {
        let fundamental = 25.0 * sine(core::f32::consts::TAU * i as f32 / 32.0);
        wave.boundaries[i] = (fundamental + if i % 2 == 0 { -70.0 } else { 70.0 }) as i16;
    }
    let mut engine = Engine::new(32, 48000);
    let mut memory = [Block::ZERO; BLOCKS];
    finish_build(&mut engine, &mut memory, wave, frequency);
    engine.phase = 0;
    engine.increment = frequency / 48000.0;
    engine.level = 1.0;
    let (mut raw_cos, mut raw_sin, mut clean_cos, mut clean_sin) = (0.0, 0.0, 0.0, 0.0);
    let (mut fundamental_cos, mut fundamental_sin) = (0.0, 0.0);
    for i in 0..FRAMES {
        let raw = wave.sample(engine.phase as f32 / 4_294_967_296.0);
        let clean = engine.tick(&memory, frequency, 48000.0, 1.0) / 5.0;
        let angle = core::f32::consts::TAU * ((i * ALIAS) % FRAMES) as f32 / FRAMES as f32;
        let cosine = sine(angle + core::f32::consts::FRAC_PI_2);
        let sinusoid = sine(angle);
        raw_cos += raw * cosine;
        raw_sin += raw * sinusoid;
        clean_cos += clean * cosine;
        clean_sin += clean * sinusoid;
        let fundamental_angle =
            core::f32::consts::TAU * ((i * FUNDAMENTAL) % FRAMES) as f32 / FRAMES as f32;
        fundamental_cos += clean * sine(fundamental_angle + core::f32::consts::FRAC_PI_2);
        fundamental_sin += clean * sine(fundamental_angle);
    }
    let raw_power = raw_cos * raw_cos + raw_sin * raw_sin;
    let clean_power = clean_cos * clean_cos + clean_sin * clean_sin;
    assert!(raw_power > 100.0);
    let amplitude_squared = 4.0
        * (fundamental_cos * fundamental_cos + fundamental_sin * fundamental_sin)
        / (FRAMES * FRAMES) as f32;
    assert!(
        (0.23 * 0.23..0.27 * 0.27).contains(&amplitude_squared),
        "fundamental squared amplitude {amplitude_squared}"
    );
    assert!(
        clean_power < raw_power * 0.0001,
        "raw {raw_power}, filtered {clean_power}"
    );
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
            wave.smoothing = smoothing;
            let mut engine = Engine::new(n, 48000);
            let mut memory = [Block::ZERO; BLOCKS];
            finish_build(&mut engine, &mut memory, wave, 20.0);
            let mut difference = 0.0;
            for (i, old) in previous.iter_mut().enumerate() {
                let sample = engine.display_sample(&memory, (i as u32) << 24);
                difference += (sample - *old) * (sample - *old);
                *old = sample;
            }
            if smoothing > 30 {
                assert!(difference / SIZE as f32 > 0.000001);
            }
        }
    }
}

#[test]
fn pitch_changes_reuse_tables_while_the_editor_shape_stays_unchanged() {
    let mut wave = Wave::new(32);
    wave.curves.fill(100);
    let mut engine = Engine::new(32, 48000);
    let mut memory = [Block::ZERO; BLOCKS];
    finish_build(&mut engine, &mut memory, wave, 20.0);
    let old_shape = engine.display_segment_sample(&memory, 0, 0.5);
    let old_audio = engine.display_sample(&memory, 1 << 28);
    let original_tables = memory;
    for frequency in [20.0, 2800.0, 12500.0] {
        for _ in 0..1000 {
            engine.tick(&memory, frequency, 48000.0, 1.0);
        }
        for _ in 0..512 {
            // Compare after the slew settles so preview and tick use the same pitch.
            let expected = engine.display_sample(&memory, engine.phase);
            let actual = engine.tick(&memory, frequency, 48000.0, 1.0);
            assert!((actual - expected * 5.0).abs() < 0.00001);
        }
        for segment in 0..wave.count {
            for point in 0..=16 {
                let t = point as f32 / 16.0;
                let phase = if segment + 1 == wave.count && point == 16 {
                    0
                } else {
                    (((segment as f32 + t) / wave.count as f32) * 4_294_967_296.0) as u32
                };
                assert_eq!(
                    engine.audio_segment_sample(&memory, segment, t),
                    engine.display_sample(&memory, phase)
                );
            }
        }
    }
    assert_eq!(engine.display_segment_sample(&memory, 0, 0.5), old_shape);
    assert!((engine.display_sample(&memory, 1 << 28) - old_audio).abs() > 0.001);
    for (old, new) in original_tables.iter().zip(&memory) {
        for (a, b) in old.data.iter().zip(&new.data) {
            assert_eq!((a.re, a.im), (b.re, b.im));
        }
    }
}

#[test]
fn editor_shows_exact_source_or_creative_smoothing_at_every_pitch() {
    for n in [2, 7, 32] {
        let mut wave = Wave::new(n);
        wave.boundaries[0] = 63;
        for i in 1..n {
            wave.boundaries[i] = if i % 2 == 0 { 100 } else { -75 };
        }
        wave.curves.fill(100);
        let mut engine = Engine::new(n, 48000);
        let mut memory = [Block::ZERO; BLOCKS];
        for smoothing in [0, 50, 100, 0] {
            wave.smoothing = smoothing;
            finish_build(&mut engine, &mut memory, wave, 20.0);
            let mut reference = [0.0; MAX_SEGMENTS * 33];
            for segment in 0..n {
                for point in 0..=32 {
                    let t = point as f32 / 32.0;
                    let value = engine.display_segment_sample(&memory, segment, t);
                    reference[segment * 33 + point] = value;
                    if smoothing == 0 {
                        assert_eq!(value, wave.segment_sample(segment, t));
                    }
                }
                assert!(
                    (engine.display_segment_sample(&memory, segment, 0.0)
                        - wave.boundary_level(segment))
                    .abs()
                        < 0.001
                );
            }
            for frequency in [2800.0, 12500.0] {
                for _ in 0..1000 {
                    engine.tick(&memory, frequency, 48000.0, 1.0);
                }
                for segment in 0..n {
                    for point in 0..=32 {
                        assert_eq!(
                            engine.display_segment_sample(&memory, segment, point as f32 / 32.0),
                            reference[segment * 33 + point]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn rebuild_crossfade_starts_at_old_wave_and_continuous_edits_do_not_starve() {
    let mut engine = Engine::new(32, 48000);
    let mut memory = [Block::ZERO; BLOCKS];
    let mut wave = Wave::new(32);
    finish_build(&mut engine, &mut memory, wave, 2800.0);
    wave.boundaries[4] = -100;
    engine.request(wave);
    for _ in 0..BUILD_STEPS {
        engine.advance(&mut memory);
    }
    assert_eq!(engine.fade, engine.fade_length);
    let phase = engine.phase;
    assert_eq!(
        engine.display_sample(&memory, phase),
        engine.wave_sample(&memory, engine.active, phase)
    );
    for block in 0..200 {
        wave.boundaries[4] = block as i16 - 100;
        engine.request(wave);
        engine.advance(&mut memory);
        for _ in 0..64 {
            assert!(engine.tick(&memory, 2800.0, 48000.0, 1.0).is_finite());
        }
    }
    assert!(engine.waves[engine.active].boundaries[4] > 70);
    assert_ne!(engine.phase, phase);
    finish_build(&mut engine, &mut memory, wave, 2800.0);
}

#[test]
fn startup_builds_default_wave_without_an_edit_and_stays_finite_at_supported_rates() {
    for rate in [32000, 44100, 48000, 88200, 96000] {
        let mut engine = Engine::new(8, rate);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut energy = 0.0;
        for _ in 0..BUILD_STEPS + 20 {
            engine.request(Wave::new(8));
            engine.advance(&mut memory);
            for _ in 0..64 {
                let sample = engine.tick(&memory, 2800.0, rate as f32, 1.0);
                assert!(sample.is_finite());
                energy += sample * sample;
            }
        }
        assert!(energy > 1.0);
        assert!(engine.ready[engine.active]);
    }
}
