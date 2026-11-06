use super::*;
extern crate std;
use std::cell::Cell;

std::thread_local! {
    static FFT_COUNTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
    static FFT_HALVES: Cell<usize> = const { Cell::new(0) };
    static GENERATED_SAMPLES: Cell<usize> = const { Cell::new(0) };
    static BOUNDARY_POSITIONS: Cell<usize> = const { Cell::new(0) };
    static OSCILLATOR_TICKS: Cell<usize> = const { Cell::new(0) };
    static MOVING_PITCH_TICKS: Cell<usize> = const { Cell::new(0) };
    static STEADY_TICKS: Cell<usize> = const { Cell::new(0) };
    static FREQUENCY_CONVERSIONS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn count_frequency_conversion() {
    FREQUENCY_CONVERSIONS.with(|counter| counter.set(counter.get() + 1));
}
pub(super) fn count_steady_tick() {
    count_oscillator_tick();
    STEADY_TICKS.with(|counter| counter.set(counter.get() + 1));
}

pub(super) fn count_oscillator_tick() {
    OSCILLATOR_TICKS.with(|counter| counter.set(counter.get() + 1));
}
pub(super) fn oscillator_tick_count() -> usize {
    OSCILLATOR_TICKS.with(|counter| counter.get())
}
pub(super) fn playback_work_counts() -> (usize, usize) {
    (
        STEADY_TICKS.with(|counter| counter.get()),
        FREQUENCY_CONVERSIONS.with(|counter| counter.get()),
    )
}

pub(super) fn count_generated_samples(count: usize) {
    GENERATED_SAMPLES.with(|counter| counter.set(counter.get() + count));
}

pub(super) fn count_boundary_position() {
    BOUNDARY_POSITIONS.with(|counter| counter.set(counter.get() + 1));
}

pub(super) fn count_fft_half() {
    FFT_HALVES.with(|counter| counter.set(counter.get() + 1));
}

fn build_calls(wave: &Wave, tables: usize) -> usize {
    audio_table_stage(wave) + tables.div_ceil(TABLES_PER_STEP) * TABLE_PAIR_STEPS
}

pub(super) fn count_fft(inverse: bool) {
    FFT_COUNTS.with(|counter| {
        let (forward, backward) = counter.get();
        counter.set((
            forward + usize::from(!inverse),
            backward + usize::from(inverse),
        ));
    });
}

fn read_table(engine: &Engine, memory: &Memory, index: usize) -> [Complex; SIZE] {
    let mut table = [Complex::default(); SIZE];
    for (i, sample) in table.iter_mut().enumerate() {
        sample.re = engine.lookup(memory, index, (i as u32) << 24);
    }
    table
}

// Independent reference retaining the original per-rebuild recurrences.
fn reference_anchor(wave: &Wave, data: &mut [Complex; SIZE], twiddles: &[Complex; SIZE]) {
    let n = wave.count;
    for _ in 0..2 {
        let mut errors = [0.0; MAX_SEGMENTS];
        for (i, error) in errors.iter_mut().take(n).enumerate() {
            let position = i as f32 * SIZE as f32 / n as f32;
            let index = position as usize & (SIZE - 1);
            let fraction = position - index as f32;
            let actual =
                data[index].re + (data[(index + 1) & (SIZE - 1)].re - data[index].re) * fraction;
            *error = wave.boundary_level(i) - actual;
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
            let step = twiddles[k];
            let (mut cosine, mut sinusoid) = (1.0, 0.0);
            for value in data.iter_mut() {
                value.re += (a * scale) * cosine + (b * scale) * sinusoid;
                let next = cosine * step.re + sinusoid * step.im;
                sinusoid = sinusoid * step.re - cosine * step.im;
                cosine = next;
            }
        }
    }
    data[0].re = wave.boundary_level(0);
}

#[test]
fn cached_smoothing_matches_original_for_every_segment_count() {
    for n in 2..=MAX_SEGMENTS {
        let mut memory = [Block::ZERO; BLOCKS];
        let mut engine = Engine::new(n, 48000);
        finish_build(&mut engine, &mut memory, Wave::new(n), 440.0);
        let cached = [memory[BANK_BLOCKS + 3], memory[BANK_BLOCKS + 4]];
        for seed in 0..8 {
            let mut wave = Wave::new(n);
            for i in 0..n {
                wave.boundaries[i] = ((i * 73 + seed * 43) % 201) as i16 - 100;
                wave.curves[i] = ((i * 31 + seed * 67) % 201) as i16 - 100;
            }
            for smoothing in [1, 25, 50, 100] {
                wave.smoothing = smoothing;
                let mut reference = [Complex::default(); SIZE];
                for (i, value) in reference.iter_mut().enumerate() {
                    value.re = wave.sample(i as f32 / SIZE as f32);
                }
                fft(&mut reference, &memory[TWIDDLES].data, false);
                for (k, value) in reference.iter_mut().enumerate() {
                    let harmonic = k.min(SIZE - k);
                    let ratio = harmonic as f32 / 4.0;
                    let filtered = 1.0 / (1.0 + ratio * ratio * ratio * ratio);
                    let slot = memory[TWIDDLES].data[SIZE / 2 + harmonic / 2];
                    assert_eq!(filtered, if harmonic & 1 == 0 { slot.re } else { slot.im });
                    let gain = 1.0 + smoothing as f32 * 0.01 * (filtered - 1.0);
                    value.re *= gain;
                    value.im *= gain;
                }
                fft(&mut reference, &memory[TWIDDLES].data, true);
                reference_anchor(&wave, &mut reference, &memory[TWIDDLES].data);
                finish_build(&mut engine, &mut memory, wave, 440.0);
                let preview =
                    read_table(&engine, &memory, engine.active * TABLES_PER_BANK + LEVELS);
                for (i, (actual, expected)) in preview.iter().zip(&reference).enumerate() {
                    assert!(
                        (actual.re - expected.re).abs() < 0.0001,
                        "segments {n}, seed {seed}, smoothing {smoothing}, sample {i}: {} vs {}",
                        actual.re,
                        expected.re
                    );
                }
                assert_eq!(engine.basis_count, n);
                for (stored, initial) in memory[BANK_BLOCKS + 3..].iter().zip(&cached) {
                    for (a, b) in stored.data.iter().zip(&initial.data) {
                        assert_eq!(a.re, b.re);
                        assert_eq!(a.im, b.im);
                    }
                }
            }
        }
    }
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
    // Allow an in-flight older shape, the requested build, and both fades.
    let longest_build = SOURCE_STEPS
        + 7
        + SAMPLE_STEPS
        + 2 * correction_steps(MAX_SEGMENTS)
        + engine.layout.count.div_ceil(TABLES_PER_STEP) * TABLE_PAIR_STEPS;
    for _ in 0..2 * longest_build + 12 {
        engine.advance(memory);
        for _ in 0..64 {
            assert!(engine.tick(memory, frequency, 48000.0, 1.0).is_finite());
        }
    }
    assert_eq!(engine.waves[engine.active], wave);
    assert_eq!(engine.fade, 0);
}

#[test]
fn waveform_boundary_presets_approximate_one_cycle_at_every_segment_count() {
    let examples = [
        (BoundaryShape::Sine, [-100, -71, 0, 71, 100, 71, 0, -71]),
        (BoundaryShape::Triangle, [-100, -50, 0, 50, 100, 50, 0, -50]),
        (
            BoundaryShape::Square,
            [-100, -100, -100, -100, 100, 100, 100, 100],
        ),
        (
            BoundaryShape::Sawtooth,
            [-100, -71, -43, -14, 14, 43, 71, 100],
        ),
    ];
    for (shape, expected) in examples {
        for (i, level) in expected.into_iter().enumerate() {
            assert_eq!(preset_boundary(shape, i, 8), level);
        }
        for n in 2..=MAX_SEGMENTS {
            let mut wave = Wave::new(n);
            for i in 0..n {
                wave.boundaries[i] = preset_boundary(shape, i, n);
            }
            assert_eq!(wave.boundaries[0], -100);
            assert_eq!(preset_boundary(shape, n, n), -100);
            assert!(wave.boundaries[..n].iter().any(|&v| v > 0));
            assert!(wave.boundaries[..n]
                .iter()
                .all(|&v| (-100..=100).contains(&v)));
            for i in 0..=SIZE {
                let value = wave.sample(i as f32 / SIZE as f32);
                assert!(value.is_finite() && (-1.0..=1.0).contains(&value));
            }
            assert_eq!(wave.sample(0.0), wave.sample(1.0));
        }
    }
}

#[test]
fn cached_geometry_matches_reference_and_does_not_overlap_fft_or_smoothing_values() {
    for n in 2..=MAX_SEGMENTS {
        let mut memory = [Block::ZERO; BLOCKS];
        let mut engine = Engine::new(n, 48000);
        engine.advance(&mut memory);
        let fixed = memory[TWIDDLES].data;
        let mut wave = Wave::new(n);
        for warp_position in [0, 1, 94, 128, 254, 255] {
            wave.warp_position = warp_position;
            for warp_amount in [-100, -50, 50, 100] {
                wave.warp_amount = warp_amount;
                prepare_geometry(&wave, &mut memory[TWIDDLES].data);
                for (a, b) in memory[TWIDDLES].data[..POSITION_CACHE].iter().zip(&fixed) {
                    assert_eq!(a.re, b.re);
                    assert_eq!(a.im, b.im);
                }
                let mut cursor = 0;
                for i in 0..SIZE {
                    let phase = i as f32 / SIZE as f32;
                    let expected = wave.segment_at(phase);
                    let actual =
                        cached_segment_at(&wave, &memory[TWIDDLES].data, phase, &mut cursor);
                    assert_eq!(actual.0, expected.0);
                    assert!((actual.1 - expected.1).abs() < 0.000001);
                }
            }
        }
    }
}

#[test]
fn warp_position_mapping_preserves_edges_order_and_segment_shapes_at_all_settings() {
    for n in 2..=MAX_SEGMENTS {
        let mut wave = Wave::new(n);
        for i in 0..n {
            wave.boundaries[i] = (i * 73 % 201) as i16 - 100;
            wave.curves[i] = (i * 31 % 201) as i16 - 100;
        }
        for warp_position in 0..=255 {
            wave.warp_position = warp_position;
            let center = warp_position as f32 / 255.0;
            for warp_amount in [-100, -50, 0, 50, 100] {
                wave.warp_amount = warp_amount;
                assert_eq!(wave.boundary_position(0), 0.0);
                assert_eq!(wave.boundary_position(n), 1.0);
                for i in 0..n {
                    let start = wave.boundary_position(i);
                    let end = wave.boundary_position(i + 1);
                    assert!(end > start && start >= 0.0 && end <= 1.0);
                    let original = i as f32 / n as f32;
                    if warp_amount == 0 {
                        assert_eq!(start, original);
                    } else if warp_amount > 0 {
                        assert!((start - center).abs() + 0.000001 >= (original - center).abs());
                    } else {
                        assert!((start - center).abs() <= (original - center).abs() + 0.000001);
                    }
                    for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                        let phase = wave.segment_phase(i, t);
                        assert!(
                            (wave.sample(phase) - wave.segment_sample(i, t)).abs() < 0.002,
                            "n {n} center {warp_position} warp_amount {warp_amount} segment {i} t {t}"
                        );
                    }
                }
                assert_eq!(wave.sample(0.0), wave.sample(1.0));
            }
        }
    }
}

#[test]
fn redistributed_shape_drives_editor_and_final_audio_spectrum_with_smoothing() {
    for n in [8, 32] {
        let mut engine = Engine::new(n, 48000);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut wave = Wave::new(n);
        for i in 0..n {
            wave.boundaries[i] = (i * 73 % 201) as i16 - 100;
            wave.curves[i] = (i * 31 % 201) as i16 - 100;
        }
        for warp_position in [0, 94, 128, 255] {
            wave.warp_position = warp_position;
            for warp_amount in [-100, -50, 50, 100] {
                wave.warp_amount = warp_amount;
                for smoothing in [0, 50, 100] {
                    wave.smoothing = smoothing;
                    finish_build(&mut engine, &mut memory, wave, 220.0);
                    let mut expected = [Complex::default(); SIZE];
                    for (i, value) in expected.iter_mut().enumerate() {
                        let phase = i as f32 / SIZE as f32;
                        value.re = engine.editor_phase_sample(&memory, phase);
                        if smoothing == 0 {
                            assert_eq!(value.re, wave.sample(phase));
                        }
                    }
                    fft(&mut expected, &memory[TWIDDLES].data, false);
                    // The spectrum used for all audio tables is the stretched
                    // shape, with pitch filtering still applied afterwards.
                    for (a, b) in memory[BANK_BLOCKS].data.iter().zip(&expected) {
                        assert!((a.re - b.re).abs() < 0.0001);
                        assert!((a.im - b.im).abs() < 0.0001);
                    }
                    for boundary in 0..n {
                        let phase = wave.boundary_position(boundary);
                        assert!(
                            (engine.editor_phase_sample(&memory, phase)
                                - wave.boundary_level(boundary))
                            .abs()
                                < 0.0005
                        );
                    }
                    let saved = engine.waves[engine.active];
                    engine.request(wave);
                    engine.advance(&mut memory);
                    assert_eq!(engine.stage, 0);
                    assert_eq!(engine.waves[engine.active], saved);
                }
            }
        }
    }
}

#[test]
fn split_fft_matches_unsplit_transform_exactly_in_both_directions() {
    let mut engine = Engine::new(32, 48000);
    let mut memory = [Block::ZERO; BLOCKS];
    engine.advance(&mut memory);
    let twiddles = &memory[TWIDDLES].data;
    for inverse in [false, true] {
        for seed in 0..8 {
            let mut expected = [Complex::default(); SIZE];
            for (i, value) in expected.iter_mut().enumerate() {
                *value = Complex {
                    re: ((i * 73 + seed * 43) % 257) as f32 / 128.0 - 1.0,
                    im: ((i * 31 + seed * 67) % 257) as f32 / 128.0 - 1.0,
                };
            }
            let mut actual = expected;
            fft(&mut expected, twiddles, inverse);
            fft_half(&mut actual, twiddles, inverse, false);
            assert!(actual
                .iter()
                .zip(&expected)
                .any(|(a, b)| a.re != b.re || a.im != b.im));
            fft_half(&mut actual, twiddles, inverse, true);
            for (a, b) in actual.iter().zip(&expected) {
                assert_eq!(a.re.to_bits(), b.re.to_bits());
                assert_eq!(a.im.to_bits(), b.im.to_bits());
            }
        }
    }
}

#[test]
fn paired_table_builds_match_separate_transforms_and_use_half_the_inverse_ffts() {
    for tables in 1..=MAX_TABLES {
        for n in [2, 7, 32] {
            for smoothing in [0, 50, 100] {
                let mut storage = [Block::ZERO; blocks(MAX_TABLES)];
                let memory = &mut storage[..blocks(tables)];
                let mut engine = Engine::with_tables(n, 48000, tables);
                let mut wave = Wave::new(n);
                wave.smoothing = smoothing;
                for i in 0..n {
                    wave.boundaries[i] = (i * 73 % 201) as i16 - 100;
                    wave.curves[i] = (i * 31 % 201) as i16 - 100;
                }
                // Exercise both physical bank alignments. With an even table
                // count, the second bank begins in an imaginary storage lane.
                for generation in 0..2 {
                    wave.boundaries[1] += generation;
                    let old = read_table(&engine, memory, engine.active * (tables + 1));
                    engine.request(wave);
                    FFT_COUNTS.with(|counter| counter.set((0, 0)));
                    let audio_stage = audio_table_stage(&wave);
                    for callback in 0..build_calls(&wave, tables) {
                        let before = FFT_COUNTS.with(|counter| counter.get());
                        let halves_before = FFT_HALVES.with(|counter| counter.get());
                        engine.advance(memory);
                        let after = FFT_COUNTS.with(|counter| counter.get());
                        let halves = FFT_HALVES.with(|counter| counter.get()) - halves_before;
                        let fft_callback = callback == SOURCE_FFT_STAGE
                            || callback == SOURCE_FFT_STAGE + 1
                            || (smoothing > 0
                                && (callback == SOURCE_STEPS + 1
                                    || callback == SOURCE_STEPS + 2
                                    || callback == audio_stage - 3
                                    || callback == audio_stage - 2))
                            || (callback >= audio_stage
                                && matches!((callback - audio_stage) % TABLE_PAIR_STEPS, 1 | 2));
                        assert_eq!(halves, usize::from(fft_callback));
                        assert!((after.0 - before.0) + (after.1 - before.1) <= 1);
                        let inverses = FFT_COUNTS.with(|counter| counter.get().1);
                        let before_smoothing =
                            usize::from(smoothing > 0 && callback >= SOURCE_STEPS + 2);
                        let audio_inverses =
                            (callback + 2).saturating_sub(audio_stage) / TABLE_PAIR_STEPS;
                        assert_eq!(inverses, before_smoothing + audio_inverses);
                    }
                    assert_eq!(engine.fade, engine.fade_length);
                    assert_eq!(engine.waves[1 - engine.active], wave);
                    let old_after_build = read_table(&engine, memory, engine.active * (tables + 1));
                    for (before, after) in old.iter().zip(&old_after_build) {
                        assert_eq!(before.re, after.re);
                    }
                    let (forward, inverse) = FFT_COUNTS.with(|counter| counter.get());
                    assert_eq!(forward, 1 + usize::from(smoothing > 0));
                    assert_eq!(inverse, tables.div_ceil(2) + usize::from(smoothing > 0));
                    // Independently reproduce the previous one-IFFT-per-table
                    // path from the same source spectrum, then compare samples.
                    let source = memory[tables + 1].data;
                    for level in 0..tables {
                        let mut reference = [Complex::default(); SIZE];
                        let limit = engine.layout.cutoff(level);
                        for (k, sample) in reference.iter_mut().enumerate() {
                            if tables == 1 || (k.min(SIZE - k) <= limit && k != SIZE / 2) {
                                *sample = source[k];
                            }
                        }
                        fft(&mut reference, &memory[tables + 3].data, true);
                        let actual =
                            read_table(&engine, memory, (1 - engine.active) * (tables + 1) + level);
                        for (i, (actual, expected)) in actual.iter().zip(&reference).enumerate() {
                            assert!((actual.re - expected.re).abs() < 0.00001,
                                "tables {tables}, segments {n}, smoothing {smoothing}, generation {generation}, level {level}, sample {i}");
                        }
                    }
                    for _ in 0..240 {
                        engine.tick(memory, 220.0, 48000.0, 1.0);
                    }
                }
            }
        }
    }
}

#[test]
fn split_smoothing_keeps_old_bank_and_coalesces_edits_until_publication() {
    for n in [2, 8, 31, 32] {
        for smoothing in [0, 1, 100] {
            let mut engine = Engine::new(n, 48000);
            let mut memory = [Block::ZERO; BLOCKS];
            let mut wave = Wave::new(n);
            wave.smoothing = smoothing;
            wave.warp_position = 37;
            wave.warp_amount = -70;
            finish_build(&mut engine, &mut memory, wave, 440.0);
            let old_bank = engine.active;
            let old = read_table(&engine, &memory, old_bank * TABLES_PER_BANK);
            wave.boundaries[0] = 70;
            wave.curves[0] = -80;
            let snapshot = wave;
            engine.request(snapshot);
            let callbacks = build_calls(&snapshot, LEVELS);
            for callback in 0..callbacks {
                GENERATED_SAMPLES.with(|counter| counter.set(0));
                BOUNDARY_POSITIONS.with(|counter| counter.set(0));
                engine.advance(&mut memory);
                GENERATED_SAMPLES.with(|counter| assert!(counter.get() <= SAMPLES_PER_STEP));
                BOUNDARY_POSITIONS.with(|counter| {
                    assert_eq!(counter.get(), if callback == 0 { n + 1 } else { 0 })
                });
                assert_eq!(engine.active, old_bank);
                assert_eq!(engine.building, snapshot);
                assert_eq!(engine.fade > 0, callback + 1 == callbacks);
                let current = read_table(&engine, &memory, old_bank * TABLES_PER_BANK);
                for (a, b) in current.iter().zip(&old) {
                    assert_eq!(a.re, b.re);
                }
                // Latest requests must not overwrite partial smoothing scratch
                // or restart the build; they belong to the following build.
                wave.boundaries[0] = -(callback as i16);
                wave.smoothing = if callback & 1 == 0 { 0 } else { 50 };
                engine.request(wave);
                if callback + 1 < callbacks {
                    for _ in 0..64 {
                        engine.tick(&memory, 440.0, 48000.0, 1.0);
                    }
                }
            }
            assert_eq!(engine.waves[1 - old_bank], snapshot);
            assert!(engine.dirty);
            finish_build(&mut engine, &mut memory, wave, 440.0);
            assert_eq!(engine.waves[engine.active], wave);
        }
    }
}

#[test]
fn cached_playback_preserves_pitch_gain_and_edit_fades_against_previous_mono_path() {
    let mut shared = Engine::new(32, 48000);
    let mut reference = shared;
    let mut memory = [Block::ZERO; BLOCKS];
    let mut reference_memory = memory;
    let mut oscillator = Oscillator::new();
    let mut wave = Wave::new(32);
    let mut pitch_error_bound = 0.0f64;
    for block in 0..160 {
        if block == 40 {
            wave.curves[2] = 75;
            wave.smoothing = 40;
        }
        if block == 100 {
            wave.warp_amount = -70;
            wave.fold_limit = 1;
        }
        shared.request(wave);
        reference.request(wave);
        shared.advance(&mut memory);
        reference.advance(&mut reference_memory);
        let frequency = if block < 80 { 220.0 } else { 8800.0 };
        for frame in 0..64 {
            reference.phase = oscillator.phase;
            let old_increment = reference.increment as f64;
            let actual = oscillator.tick(&shared, &memory, frequency, 48000.0, 0.5, frame);
            let expected = reference.tick(&reference_memory, frequency, 48000.0, 0.5);
            // Compare audio at the same phase; validate pitch separately with
            // a recurrence error budget including old f32 rounding and one
            // integer phase unit per fixed-point update.
            let alpha = (1000.0f32 / 48000.0) as f64;
            let target = (frequency / 48000.0) as f64;
            let exact = old_increment + alpha * (target - old_increment);
            let float_round = ((reference.increment as f64 - exact) * PHASE_SCALE as f64).abs();
            pitch_error_bound = (1.0 - alpha) * pitch_error_bound + float_round + 1.0 + alpha;
            let error = (oscillator.phase_step as f64
                - reference.increment as f64 * PHASE_SCALE as f64)
                .abs();
            assert!(
                error
                    <= pitch_error_bound
                        + 1.0
                        + if oscillator.pair_pending {
                            0.5 * alpha
                                * alpha
                                * (target - old_increment).abs()
                                * PHASE_SCALE as f64
                        } else {
                            0.0
                        },
                "pitch error {error} > {pitch_error_bound}"
            );
            assert_eq!(oscillator.level, reference.level);
            assert!(
                (actual - expected).abs() < 0.15,
                "block {block}, frame {frame}: {actual} vs {expected}"
            );
        }
        shared.advance_time(64);
        shared.show_voice(&oscillator);
        assert_eq!(shared.fade, reference.fade);
        assert_eq!(shared.active, reference.active);
    }
}

#[test]
fn anti_alias_cache_ignores_small_jitter_but_pitch_tracks_it_and_slow_changes_accumulate() {
    let engine = Engine::new(8, 48000);
    let memory = [Block::ZERO; BLOCKS];
    let mut voice = Oscillator::new();
    // In the blend band for the richest table, where small pitch changes would
    // otherwise change the blend on every sample.
    let base = 48000.0 * 0.48 / 127.0;
    voice.tune_when_silent(base, 48000.0);
    voice.tick(&engine, &memory, base, 48000.0, 1.0, 0);
    let accepted = voice.selected_increment();
    let selection = (voice.playback_level, voice.playback_blend);
    let start_phase = voice.phase;
    let mut reference = Oscillator::new();
    reference.tune_when_silent(base, 48000.0);
    reference.tick(&engine, &memory, base, 48000.0, 1.0, 0);
    for frame in 0..2000 {
        let frequency = base * if frame % 100 < 50 { 1.0003 } else { 0.9997 };
        voice.tick(&engine, &memory, frequency, 48000.0, 1.0, frame);
        reference.tick(&engine, &memory, base, 48000.0, 1.0, frame);
        assert_eq!(voice.selected_increment(), accepted);
        assert_eq!((voice.playback_level, voice.playback_blend), selection);
    }
    assert_ne!(voice.pitch_increment(), accepted);
    assert_ne!(voice.phase, reference.phase);
    assert_ne!(voice.phase, start_phase);
    // Tiny monotonic changes eventually cross the last accepted pitch, rather
    // than being lost because each individual change is below the threshold.
    for direction in [1.0f32, -1.0] {
        voice.tune_when_silent(base, 48000.0);
        voice.tick(&engine, &memory, base, 48000.0, 1.0, 0);
        let accepted = voice.selected_increment();
        for frame in 0..600 {
            let frequency = base * (1.0 + direction * frame as f32 * 0.000002);
            voice.tick(&engine, &memory, frequency, 48000.0, 1.0, frame);
        }
        assert!((voice.selected_increment() - accepted) * direction > 0.0);
        assert!((voice.playback_blend - selection.1) * direction > 0.0);
    }
}

#[test]
fn anti_alias_cache_refreshes_on_midi_retuning_and_stays_within_one_millivolt_on_sweeps() {
    for rate in [32000, 48000, 96000] {
        let engine = Engine::new(8, rate);
        let memory = [Block::ZERO; BLOCKS];
        let mut voice = Oscillator::new();
        for note in [36.0f32, 108.0, 60.0] {
            let frequency = 440.0 * 2.0f32.powf((note - 69.0) / 12.0);
            voice.tune_when_silent(frequency, rate as f32);
            voice.tick(&engine, &memory, frequency, rate as f32, 1.0, 0);
            assert_eq!(voice.selected_increment(), voice.pitch_increment());
            assert_eq!(
                (voice.playback_level, voice.playback_blend),
                engine.layout.playback_levels(voice.pitch_increment())
            );
        }
        for frame in 0..16000 {
            let octave = if frame < 8000 {
                frame as f32 / 1000.0
            } else {
                (16000 - frame) as f32 / 1000.0
            };
            let frequency = 40.0 * 2.0f32.powf(octave);
            voice.tick(&engine, &memory, frequency, rate as f32, 1.0, frame);
            let error = (voice.pitch_increment() / voice.selected_increment())
                .log2()
                .abs();
            assert!(error <= 0.001001, "{error} octaves at {rate} Hz");
            assert_eq!(
                (voice.playback_level, voice.playback_blend),
                engine.layout.playback_levels(voice.selected_increment())
            );
            let highest = engine.layout.cutoff(voice.playback_level) as f32;
            assert!(voice.pitch_increment() * highest < 0.5 || voice.playback_blend == 1.0);
        }
    }
}

#[test]
fn block_playback_preserves_samples_and_state_through_pitch_level_changes_and_bank_swaps() {
    for rate in [32000, 48000, 96000] {
        let mut engine = Engine::new(8, rate);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut wave = Wave::new(8);
        let mut optimized = Oscillator::new();
        let mut reference = optimized;
        let before = playback_work_counts().0;
        for block in 0..750 {
            // Settle both in a table blend and in single-table regions.
            let frequency = match block {
                0..=249 => rate as f32 * 0.48 / 127.0,
                250..=499 => 8800.0,
                _ => 220.0,
            };
            let level = if block < 370 { 0.5 } else { 0.23 };
            if block == 130 || block == 600 {
                wave.curves[0] = if block == 130 { 85 } else { -73 };
                wave.smoothing = 40;
                wave.warp_amount = -50;
            }
            engine.request(wave);
            engine.advance(&mut memory);
            optimized.prepare(frequency, rate as f32, level);
            let mut actual = [1.25; 24];
            if !optimized.render_open_gate(&engine, &memory, &mut actual) {
                for (frame, sample) in actual.iter_mut().enumerate() {
                    *sample += optimized.tick_prepared(&engine, &memory, frame);
                }
            }
            for (frame, &actual) in actual.iter().enumerate() {
                let expected =
                    1.25 + reference.tick(&engine, &memory, frequency, rate as f32, level, frame);
                let bound = 4.0 * f32::EPSILON * expected.abs().max(1.0);
                assert!(
                    (actual - expected).abs() <= bound,
                    "rate {rate}, block {block}, frame {frame}: {actual} != {expected}"
                );
            }
            assert_eq!(optimized.phase, reference.phase);
            assert_eq!(optimized.phase_step, reference.phase_step);
            assert_eq!(optimized.level, reference.level);
            engine.advance_time(24);
        }
        assert!(playback_work_counts().0 > before + 2400);
    }
}

#[test]
fn frequency_conversion_cache_persists_and_exact_cv_changes_invalidate_it() {
    let mut voice = Oscillator::new();
    let before = playback_work_counts().1;
    assert_eq!(voice.frequency(0.0), 440.0);
    let snapshot = voice;
    for _ in 0..1000 {
        assert_eq!(voice.frequency(0.0), 440.0);
    }
    assert_eq!(playback_work_counts().1 - before, 1);
    assert_eq!(voice, snapshot);
    let tiny_cv_change = 0.00001;
    assert!(voice.frequency(tiny_cv_change) > 440.0);
    assert_eq!(playback_work_counts().1 - before, 2);
    voice.frequency(tiny_cv_change);
    assert_eq!(playback_work_counts().1 - before, 2);
    assert_eq!(voice.frequency(-1.0), 220.0);
    assert_eq!(playback_work_counts().1 - before, 3);
}

#[test]
fn shared_rebuilds_and_crossfade_time_do_not_depend_on_enabled_voice_count() {
    for enabled in [0, 1, 4, 8] {
        let mut engine = Engine::new(32, 48000);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut voices = [Oscillator::new(); super::super::VOICE_COUNT];
        let mut wave = Wave::new(32);
        wave.curves.fill(73);
        wave.smoothing = 50;
        wave.warp_amount = -70;
        let callbacks = build_calls(&wave, LEVELS);
        FFT_COUNTS.with(|counter| counter.set((0, 0)));
        for callback in 0..callbacks + 5 {
            engine.request(wave);
            engine.advance(&mut memory);
            for (index, voice) in voices.iter_mut().take(enabled).enumerate() {
                for frame in 0..64 {
                    assert!(voice
                        .tick(
                            &engine,
                            &memory,
                            220.0 * (index + 1) as f32,
                            48000.0,
                            0.5,
                            frame
                        )
                        .is_finite());
                }
            }
            if callback + 1 == callbacks {
                assert_eq!(engine.fade, engine.fade_length);
            }
            engine.advance_time(64);
        }
        assert_eq!(engine.waves[engine.active], wave);
        assert_eq!(engine.fade, 0);
        assert_eq!(
            FFT_COUNTS.with(|counter| counter.get()),
            (2, LEVELS.div_ceil(2) + 1)
        );
        for voice in voices.iter().skip(enabled) {
            assert_eq!(*voice, Oscillator::new());
        }
        if enabled > 1 {
            assert_ne!(voices[0].phase, voices[1].phase);
        }
    }
}

#[test]
fn fold_limit_controls_excursion_and_preserves_bounded_curves_and_anchors() {
    let mut wave = Wave::new(3);
    for limit in 0..=MAX_FOLDS {
        wave.fold_limit = limit;
        for start in [-100, -40, 0, 45, 100] {
            for end in [-100, -40, 0, 45, 100] {
                wave.boundaries[1] = start;
                wave.boundaries[2] = end;
                for curve in [-100, -75, -25, 0, 25, 75, 100] {
                    wave.curves[1] = curve;
                    assert_eq!(wave.segment_sample(1, 0.0), wave.boundary_level(1));
                    assert_eq!(wave.segment_sample(1, 1.0), wave.boundary_level(2));
                    for i in 1..256 {
                        let t = i as f32 / 256.0;
                        let a = wave.boundary_level(1);
                        let b = wave.boundary_level(2);
                        let bend = curve as f32 * 0.01;
                        let actual = wave.segment_sample(1, t);
                        assert!(actual.is_finite() && (-1.000001..=1.000001).contains(&actual));
                        let expected = if limit == 0 {
                            let middle = (a + b) * 0.5;
                            let control = middle
                                + bend
                                    * if bend >= 0.0 {
                                        1.0 - middle
                                    } else {
                                        1.0 + middle
                                    };
                            (1.0 - t) * (1.0 - t) * a + 2.0 * (1.0 - t) * t * control + t * t * b
                        } else {
                            let mut value =
                                a + (b - a) * t + (8.0 * limit as f32) * bend * t * (1.0 - t);
                            let mut reflections = 0;
                            while value > 1.0 || value < -1.0 {
                                value = if value > 1.0 {
                                    2.0 - value
                                } else {
                                    -2.0 - value
                                };
                                reflections += 1;
                                assert!(reflections <= limit);
                            }
                            value
                        };
                        assert!((actual - expected).abs() < 0.000002);
                        if limit == DEFAULT_FOLD_LIMIT {
                            let mut legacy = a + (b - a) * t + 16.0 * bend * t * (1.0 - t);
                            for _ in 0..2 {
                                if legacy > 1.0 {
                                    legacy = 2.0 - legacy;
                                } else if legacy < -1.0 {
                                    legacy = -2.0 - legacy;
                                }
                            }
                            assert_eq!(actual.to_bits(), legacy.to_bits());
                        }
                    }
                }
            }
        }
        if limit > 0 {
            wave.boundaries[1] = 100;
            wave.boundaries[2] = 100;
            wave.curves[1] = 100;
            assert_eq!(
                wave.segment_sample(1, 0.5),
                if limit % 2 == 0 { 1.0 } else { -1.0 }
            );
        }
    }
    wave.fold_limit = 0;
    wave.boundaries[1] = 0;
    wave.boundaries[2] = 0;
    wave.curves[1] = 100;
    assert_eq!(wave.segment_sample(1, 0.5), 0.5);
}

#[test]
fn changing_fold_limit_rebuilds_the_audio_and_editor_without_changing_other_shape_values() {
    let mut engine = Engine::with_tables(8, 48000, 1);
    let mut storage = [Block::ZERO; blocks(1)];
    let mut wave = Wave::new(8);
    wave.curves.fill(73);
    let mut previous = None;
    for limit in [0, 1, 2, 0] {
        wave.fold_limit = limit;
        finish_build(&mut engine, &mut storage, wave, 220.0);
        let actual = read_table(&engine, &storage, engine.active * 2);
        for (i, value) in actual.iter().enumerate() {
            let phase = i as f32 / SIZE as f32;
            assert!((value.re - wave.sample(phase)).abs() < 0.00001);
            assert_eq!(
                engine.editor_phase_sample(&storage, phase),
                wave.sample(phase)
            );
        }
        if let Some(before) = previous {
            assert!(actual
                .iter()
                .zip(&before)
                .any(|(a, b): (&Complex, &Complex)| (a.re - b.re).abs() > 0.01));
        }
        previous = Some(actual);
        assert_eq!(engine.waves[engine.active], wave);
    }
}

#[test]
fn curve_bows_past_segment_endpoints_and_folds_only_at_global_limits() {
    let mut wave = Wave::new(3);
    wave.boundaries[1] = 20;
    wave.boundaries[2] = 40;
    wave.curves[1] = 0;
    assert!((wave.segment_sample(1, 0.5) - 0.3).abs() < 0.000001);
    wave.curves[1] = 10;
    // +70% exceeds both endpoints but stays below +100%: no reflection yet.
    assert!((wave.segment_sample(1, 0.5) - 0.7).abs() < 0.000001);
    wave.curves[1] = 30;
    // Unfolded +150% reflects at +100%, returning +50%, not at +40%.
    assert!((wave.segment_sample(1, 0.5) - 0.5).abs() < 0.000001);
    wave.boundaries[1] = -20;
    wave.boundaries[2] = -40;
    wave.curves[1] = -30;
    assert!((wave.segment_sample(1, 0.5) + 0.5).abs() < 0.000001);
    wave.boundaries[1] = 0;
    wave.boundaries[2] = 0;
    // Equal endpoints can now form a hump and multiple reflected lobes.
    for (curve, midpoint) in [
        (10, 0.4),
        (-10, -0.4),
        (25, 1.0),
        (50, 0.0),
        (75, -1.0),
        (100, 0.0),
    ] {
        wave.curves[1] = curve;
        assert!((wave.segment_sample(1, 0.5) - midpoint).abs() < 0.000001);
    }
}

#[test]
fn folded_curves_preserve_endpoints_range_and_continuity() {
    let mut wave = Wave::new(3);
    for start in [-100, -40, 0, 45, 100] {
        for end in [-100, -40, 0, 45, 100] {
            wave.boundaries[1] = start;
            wave.boundaries[2] = end;
            let mut previous_curve = [0.0; 129];
            for curve in -100..=100 {
                wave.curves[1] = curve;
                assert_eq!(wave.segment_sample(1, 0.0), wave.boundary_level(1));
                assert_eq!(wave.segment_sample(1, 1.0), wave.boundary_level(2));
                let mut previous_sample = wave.segment_sample(1, 0.0);
                for (i, last) in previous_curve.iter_mut().enumerate() {
                    let value = wave.segment_sample(1, i as f32 / 128.0);
                    assert!((-1.000001..=1.000001).contains(&value));
                    // The unfolded slope is at most 18; reflection preserves
                    // continuity and cannot increase this per-step bound.
                    assert!((value - previous_sample).abs() <= 18.0 / 128.0 + 0.00001);
                    if curve > -100 {
                        assert!((value - *last).abs() <= 0.04001);
                    }
                    if curve == 0 {
                        let expected = wave.boundary_level(1)
                            + (wave.boundary_level(2) - wave.boundary_level(1)) * i as f32 / 128.0;
                        assert!((value - expected).abs() < 0.000001);
                    }
                    *last = value;
                    previous_sample = value;
                }
            }
        }
    }
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
            for _ in 0..build_calls(&wave, tables) + 8 {
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
    // The audible bank may be one completed generation behind the pending
    // generation. Bound the lag by two build/fade cycles, rather than the old
    // fixed threshold that assumed a much shorter callback schedule.
    let max_lag = 2 * (BUILD_STEPS + engine.fade_length.div_ceil(64));
    assert!(engine.waves[engine.active].boundaries[4] >= wave.boundaries[4] - max_lag as i16);
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

pub(super) fn count_moving_pitch_tick() {
    count_oscillator_tick();
    MOVING_PITCH_TICKS.with(|counter| counter.set(counter.get() + 1));
}

#[test]
fn moving_pitch_renderer_preserves_state_and_bounded_samples_across_pitch_changes() {
    for rate in [32000, 48000, 96000] {
        let mut engine = Engine::new(32, rate);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut wave = Wave::new(32);
        wave.smoothing = 17;
        wave.curves[3] = 85;
        finish_build(&mut engine, &mut memory, wave, 440.0);
        let mut optimized = Oscillator::new();
        let mut reference = optimized;
        // Settle level and pitch before exercising the moving-pitch-only path.
        for frame in 0..2400 {
            optimized.tick(&engine, &memory, 440.0, rate as f32, 0.5, frame);
            reference.tick(&engine, &memory, 440.0, rate as f32, 0.5, frame);
        }
        let before = MOVING_PITCH_TICKS.with(|counter| counter.get());
        for block in 0..1000 {
            let frequency = match block {
                0..=249 => 440.0 + (block % 3) as f32 * 0.0001,
                250..=749 => 20.0 * (2.0f32).powf((block - 250) as f32 / 50.0),
                _ => 20000.0 * (0.5f32).powf((block - 750) as f32 / 25.0),
            };
            optimized.prepare(frequency, rate as f32, 0.5);
            let mut output = [0.125; 24];
            assert!(optimized.render_open_gate(&engine, &memory, &mut output));
            for (frame, sample) in output.iter().enumerate() {
                let expected =
                    0.125 + reference.tick(&engine, &memory, frequency, rate as f32, 0.5, frame);
                let bound = 4.0 * f32::EPSILON * expected.abs().max(1.0);
                assert!(
                    (sample - expected).abs() <= bound,
                    "rate {rate}, block {block}, frame {frame}: {sample} != {expected}"
                );
            }
            assert_eq!(optimized, reference);
        }
        assert!(MOVING_PITCH_TICKS.with(|counter| counter.get()) > before + 20000);
    }
}

#[test]
fn precomputed_gain_tracks_original_scaling_with_bounded_rounding_error() {
    let mut max_error = 0.0f32;
    for rate in [32000, 48000, 96000] {
        let mut engine = Engine::new(32, rate);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut wave = Wave::new(32);
        wave.smoothing = 40;
        wave.warp_amount = -60;
        wave.curves[3] = 90;
        finish_build(&mut engine, &mut memory, wave, 440.0);
        for db in -60..=0 {
            let level = if db == -60 {
                0.0
            } else {
                10.0f32.powf(db as f32 / 20.0)
            };
            for frequency in [20.0, 440.0, 8800.0, 20000.0] {
                let mut actual = Oscillator::new();
                actual.level = level;
                actual.target_level = level;
                actual.tune_when_silent(frequency, rate as f32);
                actual.prepare(frequency, rate as f32, level);
                let mut reference = actual;
                for _ in 0..4 {
                    let mut output = [0.0; 64];
                    assert!(actual.render_open_gate(&engine, &memory, &mut output));
                    for (frame, &sample) in output.iter().enumerate() {
                        let expected = reference.tick_prepared(&engine, &memory, frame);
                        max_error = max_error.max((sample - expected).abs());
                        let bound = 4.0 * f32::EPSILON * expected.abs().max(1.0);
                        assert!(
                            (sample - expected).abs() <= bound,
                            "rate {rate}, dB {db}, frequency {frequency}: {sample} != {expected}"
                        );
                    }
                    assert_eq!(actual, reference);
                }
            }
        }
    }
    std::println!("Maximum gain-regrouping sample error: {max_error} V");
}

#[test]
fn fixed_pitch_tracks_exponential_reference_without_overshoot_and_settles_exactly() {
    for rate in [32000, 44100, 48000, 96000] {
        let mut voice = Oscillator::new();
        for frequency in [20.0, 440.0, 12000.0, 20.0, 20.0001, 19.9999] {
            voice.prepare(frequency, rate as f32, 1.0);
            let mut exact = voice.phase_step as f64;
            let alpha = voice.pitch_slew as f64 / PITCH_SLEW_SCALE as f64;
            for _ in 0..5000 {
                let before = voice.phase_step;
                exact += alpha * (voice.target_step as f64 - exact);
                voice.advance_pitch_step_exact();
                assert!((voice.phase_step as f64 - exact).abs() <= 1.0 / alpha + 2.0);
                assert!(
                    (before.min(voice.target_step)..=before.max(voice.target_step))
                        .contains(&voice.phase_step)
                );
            }
            assert_eq!(voice.phase_step, voice.target_step);
            assert!(!voice.pitch_moving);
            let actual_hz = voice.phase_step as f64 * rate as f64 / PHASE_SCALE as f64;
            let cents = 1200.0 * (actual_hz / frequency as f64).log2().abs();
            assert!(cents < 0.002, "{rate} Hz, {frequency} Hz: {cents} cents");
        }
    }
}

#[test]
fn fixed_pitch_follows_sub_millivolt_ramps_and_reprepares_after_rate_changes() {
    let mut voice = Oscillator::new();
    for rate in [32000, 48000, 96000, 44100] {
        let engine = Engine::new(8, rate);
        let base = 220.0;
        voice.tune_when_silent(base, rate as f32);
        voice.prepare(base, rate as f32, 1.0);
        voice.advance_pitch(&engine);
        assert_eq!(voice.phase_step, voice.target_step);
        for direction in [1.0, -1.0] {
            for frame in 1..1000 {
                // Each target change is about 0.003 mV at 1V/oct.
                let frequency = base * (1.0 + direction * frame as f32 * 0.000002);
                let before = voice.phase_step;
                voice.prepare(frequency, rate as f32, 1.0);
                voice.advance_pitch(&engine);
                if frame > 100 {
                    assert!((voice.phase_step as i64 - before as i64) as f32 * direction > 0.0);
                }
            }
        }
    }
}

#[test]
fn shared_cv_factor_preserves_tuning_clamps_and_reuses_conversion_across_detuned_voices() {
    let mut cache = CvPitchCache::new();
    let mut voices = [Oscillator::new(); 10];
    let mut max_cents = 0.0f64;
    for bus in [1, 64, 0, 3] {
        for voltage in [-10.0, -3.14159, -0.00001, 0.0, 0.00001, 2.71828, 10.0] {
            cache.begin_block();
            let before = playback_work_counts().1;
            for (i, voice) in voices.iter_mut().enumerate() {
                let tuning = i as f32 * 2.1 - 9.1;
                let factor = cache.factor(bus, voltage);
                let actual = voice.cv_frequency(tuning, factor);
                let expected = 440.0 * 2.0f64.powf(((tuning + voltage).clamp(-16.0, 16.0)) as f64);
                max_cents = max_cents.max(1200.0 * (actual as f64 / expected).log2().abs());
            }
            // Ten fixed detune factors plus one CV factor at first, only one
            // conversion for later CV changes (or zero for exact zero volts).
            assert!(playback_work_counts().1 - before <= if voltage == 0.0 { 10 } else { 11 });
            cache.begin_block();
            let before = playback_work_counts().1;
            for (i, voice) in voices.iter_mut().enumerate() {
                voice.cv_frequency(i as f32 * 2.1 - 9.1, cache.factor(bus, voltage));
            }
            assert_eq!(playback_work_counts().1, before);
        }
    }
    assert!(max_cents < 0.002, "{max_cents} cents");
    std::println!("Maximum factored tuning difference: {max_cents} cents");
}

#[test]
fn fixed_selection_proof_is_strict_and_tracks_identically_to_scalar_on_both_sides() {
    let engine = Engine::new(8, 48000);
    let mut voice = Oscillator::new();
    voice.selection_low = 1000;
    voice.selection_high = 2000;
    for current in [999, 1000, 1001, 1500, 1999, 2000, 2001] {
        for target in [999, 1000, 1001, 1500, 1999, 2000, 2001] {
            voice.phase_step = current;
            voice.target_step = target;
            assert_eq!(
                voice.selection_covers_target(),
                current > 1000 && current < 2000 && target > 1000 && target < 2000
            );
        }
    }
    for frequency in [100.0, 440.0, 3000.0, 9000.0, 19000.0] {
        let mut engine = engine;
        let mut memory = [Block::ZERO; BLOCKS];
        finish_build(&mut engine, &mut memory, Wave::new(8), frequency);
        for direction in [-1.0, 1.0] {
            let mut fast = Oscillator::new();
            fast.tune_when_silent(frequency, 48000.0);
            fast.prepare(frequency, 48000.0, 0.5);
            for _ in 0..2000 {
                fast.tick_prepared(&engine, &memory, 0);
            }
            fast.prepare(frequency * (1.0 + direction * 0.0001), 48000.0, 0.5);
            assert!(fast.selection_covers_target());
            let mut scalar = fast;
            let mut samples = [0.0f32; 24];
            assert!(fast.render_open_gate(&engine, &memory, &mut samples));
            for (i, actual) in samples.iter().enumerate() {
                let expected = scalar.tick_prepared(&engine, &memory, i);
                assert!((actual - expected).abs() <= 4.0 * f32::EPSILON * expected.abs().max(1.0));
            }
            assert_eq!(fast, scalar);
        }
    }
}

#[test]
fn direct_cv_targets_match_frequency_path_with_bounded_rounding_across_rates_and_limits() {
    let mut max_cents = 0.0f64;
    for rate in [32000.0, 44100.0, 48000.0, 96000.0] {
        for tuning in [-10.0, -3.25, 0.0, 4.125, 9.0] {
            let mut direct = Oscillator::new();
            let mut original = Oscillator::new();
            for i in 0..=4000 {
                let cv = -10.0 + i as f32 * 0.005;
                let factor = 2.0f32.powf(cv);
                direct.prepare_cv(tuning, factor, rate, 0.5);
                let frequency = original.cv_frequency(tuning, factor);
                original.prepare(frequency, rate, 0.5);
                let expected = original.target_step;
                let error = direct.target_step.abs_diff(expected) as f64;
                assert!(error <= 4.0f64.max(expected as f64 * 4.0 * f32::EPSILON as f64));
                // Below 20 Hz, compare the phase-unit bound above: a single
                // pre-existing phase LSB is relatively large in cents there.
                if expected as f64 * rate as f64 / PHASE_SCALE as f64 >= 20.0 {
                    max_cents = max_cents
                        .max(1200.0 * (direct.target_step as f64 / expected as f64).log2().abs());
                }
                assert_eq!(direct.target_level, original.target_level);
                assert_eq!(direct.slew, original.slew);
                assert_eq!(direct.pitch_slew, original.pitch_slew);
            }
        }
    }
    assert!(max_cents < 0.002, "{max_cents} cents");
    std::println!("Maximum direct-target tuning difference: {max_cents} cents");
}

#[test]
fn direct_cv_scale_rebuilds_for_tuning_rate_and_path_changes_without_a_deadband() {
    let mut voice = Oscillator::new();
    for (tuning, rate) in [
        (0.0, 48000.0),
        (1.0, 48000.0),
        (1.0, 96000.0),
        (-2.0, 32000.0),
    ] {
        voice.prepare_cv(tuning, 1.0, rate, 0.5);
        let initial = voice.target_step;
        let scale = voice.cv_phase_scale;
        voice.prepare_cv(tuning, 1.0000005, rate, 0.5);
        assert!(voice.target_step > initial);
        assert_eq!(voice.cv_phase_scale, scale);
        voice.prepare(220.0, rate, 0.5);
        voice.prepare_cv(tuning, 1.0, rate, 0.5);
        assert_eq!(voice.target_step, initial);
    }
}

#[test]
fn paired_pitch_endpoints_track_exact_reference_monotonically_with_bounded_phase_error() {
    for rate in [32000.0, 44100.0, 48000.0, 96000.0] {
        for (start, target) in [
            (20.0, 20000.0),
            (20000.0, 20.0),
            (440.0, 441.0),
            (440.0, 440.0001),
        ] {
            let mut paired = Oscillator::new();
            paired.tune_when_silent(start, rate);
            paired.prepare(target, rate, 0.5);
            let mut exact = paired;
            let alpha = paired.pitch_slew as f64 / PITCH_SLEW_SCALE as f64;
            let distance = paired.target_step.abs_diff(paired.phase_step) as f64;
            let mut sum = 0i64;
            for _ in 0..5000 {
                let before = paired.phase_step;
                paired.advance_pitch_step();
                exact.advance_pitch_step_exact();
                assert!(
                    (before.min(paired.target_step)..=before.max(paired.target_step))
                        .contains(&paired.phase_step)
                );
                let tolerance = 3.0 / alpha
                    + 8.0
                    + if paired.pair_pending {
                        0.5 * alpha * alpha * distance
                    } else {
                        0.0
                    };
                assert!((paired.phase_step as f64 - exact.phase_step as f64).abs() <= tolerance);
                sum += paired.phase_step as i64 - exact.phase_step as i64;
            }
            assert_eq!(paired.phase_step, paired.target_step);
            assert!(!paired.pitch_moving);
            assert!(!paired.pair_pending);
            let theoretical = distance * alpha / (2.0 * (2.0 - alpha));
            assert!((sum as f64).abs() <= theoretical + 0.001 * PHASE_SCALE as f64);
        }
    }
}

#[test]
fn new_pitch_target_and_silent_retune_discard_pending_interpolation_endpoint() {
    let mut voice = Oscillator::new();
    voice.prepare(440.0, 48000.0, 0.5);
    voice.advance_pitch_step();
    assert!(voice.pair_pending);
    voice.prepare(220.0, 48000.0, 0.5);
    assert!(!voice.pair_pending);
    voice.advance_pitch_step();
    assert!(voice.pair_pending);
    voice.prepare_cv(0.0, 1.1, 48000.0, 0.5);
    assert!(!voice.pair_pending);
    voice.advance_pitch_step();
    assert!(voice.pair_pending);
    voice.tune_when_silent(880.0, 48000.0);
    assert!(!voice.pair_pending);
}
