use super::*;
extern crate std;
use std::cell::Cell;

std::thread_local! {
    static FFT_COUNTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
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

pub(super) fn count_fft(inverse: bool) {
    FFT_COUNTS.with(|counter| {
        let (forward, backward) = counter.get();
        counter.set((
            forward + usize::from(!inverse),
            backward + usize::from(inverse),
        ));
    });
}

pub(super) fn count_moving_pitch_tick() {
    count_oscillator_tick();
    MOVING_PITCH_TICKS.with(|counter| counter.set(counter.get() + 1));
}

// Fixtures drive the actual worker and bank publication code.
const BLOCKS: usize = PLAYBACK_BLOCK_COUNT;
impl Engine {
    pub fn new(count: usize, sample_rate: u32) -> Self {
        let wave = Wave::new(count);
        Self {
            layout: TableLayout::new(DEFAULT_TABLES),
            requested: wave,
            waves: [wave; 2],
            active: 0,
            dirty: true,
            fade: 0,
            fade_length: (sample_rate as usize / 200).max(1),
            ready: [false; 2],
        }
    }
}
fn build_pending(engine: &mut Engine, memory: &mut Memory) {
    if !engine.dirty || engine.fade != 0 {
        return;
    }
    crate::tests::with_build_job(engine.requested.count, |job| {
        assert!(engine.service_job(job, memory));
        job.build();
        assert!(!engine.service_job(job, memory));
    });
}
fn finish_build(engine: &mut Engine, memory: &mut Memory, wave: Wave) {
    engine.advance_time(engine.fade_length);
    engine.update_request(|request| *request = wave);
    build_pending(engine, memory);
    engine.advance_time(engine.fade_length);
    assert_eq!(engine.waves[engine.active], wave);
    assert!(!engine.dirty);
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
            engine.update_request(|request| *request = wave);
            build_pending(&mut engine, &mut memory);
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
fn moving_pitch_renderer_preserves_state_and_bounded_samples_across_pitch_changes() {
    for rate in [32000, 48000, 96000] {
        let mut engine = Engine::new(32, rate);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut wave = Wave::new(32);
        wave.smoothing = 17;
        wave.curves[3] = 85;
        finish_build(&mut engine, &mut memory, wave);
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
fn block_gain_matches_production_scalar_playback_with_bounded_rounding_error() {
    let mut max_error = 0.0f32;
    for rate in [32000, 48000, 96000] {
        let mut engine = Engine::new(32, rate);
        let mut memory = [Block::ZERO; BLOCKS];
        let mut wave = Wave::new(32);
        wave.smoothing = 40;
        wave.warp_amount = -60;
        wave.curves[3] = 90;
        finish_build(&mut engine, &mut memory, wave);
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
        finish_build(&mut engine, &mut memory, Wave::new(8));
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
                let frequency = 440.0 * 2.0f32.powf(tuning) * factor;
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

#[test]
fn batched_envelope_playback_matches_scalar_samples_phase_and_gate_energy() {
    let mut wave = Wave::new(32);
    wave.curves.fill(87);
    let mut engine = Engine::new(32, 48000);
    let mut memory = [Block::ZERO; BLOCKS];
    finish_build(&mut engine, &mut memory, wave);
    for frequency in [20.0, 440.0, 2800.0, 12500.0] {
        let mut initial = Oscillator::new();
        for _ in 0..5000 {
            initial.prepare(frequency, 48000.0, 0.5);
            initial.tick_prepared(&engine, &memory, 0);
        }
        for target in [frequency, frequency * 1.001, frequency * 2.0] {
            for silent_at in 0..=16 {
                for attacking in [false, true] {
                    let mut fast = initial;
                    fast.prepare(target, 48000.0, 0.5);
                    let mut scalar = fast;
                    let mut gates = [0.0; 16];
                    for (i, gate) in gates.iter_mut().enumerate().take(silent_at) {
                        *gate = if attacking {
                            (i + 1) as f32 / 16.0
                        } else {
                            1.0 - i as f32 / 16.0
                        };
                    }
                    let mut expected = [0.0; 16];
                    let mut expected_power = 0.0;
                    for (i, sample) in expected.iter_mut().enumerate().take(silent_at) {
                        *sample = gates[i] * scalar.tick_prepared(&engine, &memory, i);
                        expected_power += gates[i] * gates[i];
                    }
                    let before = STEADY_TICKS.with(|count| count.get());
                    let mut actual = [0.0; 16];
                    let power = fast
                        .render_enveloped(&engine, &memory, &mut actual, &gates)
                        .unwrap();
                    assert_eq!(power, expected_power);
                    assert_eq!(fast, scalar, "phase/pitch must stop at first silent gate");
                    for (actual, expected) in actual.iter().zip(expected) {
                        assert!((actual - expected).abs() < 0.000001);
                    }
                    if target == frequency {
                        assert_eq!(STEADY_TICKS.with(|count| count.get()) - before, silent_at);
                    }
                }
            }
        }
    }
}

#[test]
fn batched_envelope_rejection_does_not_advance_or_modify_output() {
    let mut engine = Engine::new(8, 48000);
    let mut memory = [Block::ZERO; BLOCKS];
    finish_build(&mut engine, &mut memory, Wave::new(8));
    let mut voice = Oscillator::new();
    voice.prepare(440.0, 48000.0, 0.5);
    let before = voice;
    let mut samples = [0.0; 16];
    assert_eq!(
        voice.render_enveloped(&engine, &memory, &mut samples, &[1.0; 15]),
        None
    );
    assert_eq!(voice, before);
    assert_eq!(
        voice.render_enveloped(&engine, &memory, &mut samples, &[1.0; 16]),
        None
    );
    assert_eq!(voice, before);
    assert_eq!(samples, [0.0; 16]);
    for _ in 0..5000 {
        voice.tick_prepared(&engine, &memory, 0);
    }
    engine.fade = 100;
    let before = voice;
    assert_eq!(
        voice.render_enveloped(&engine, &memory, &mut samples, &[1.0; 16]),
        None
    );
    assert_eq!(voice, before);
    assert_eq!(samples, [0.0; 16]);
}

#[test]
fn unwarped_sampling_matches_zero_warp_copy_without_changing_source() {
    for count in 2..=MAX_SEGMENTS {
        let mut wave = Wave::new(count);
        for i in 0..count {
            wave.boundaries[i] = ((i * 37 % 201) as i16) - 100;
            wave.curves[i] = ((i * 61 % 201) as i16) - 100;
        }
        for amount in [-100, -1, 0, 1, 100] {
            wave.warp_amount = amount;
            for fold in 0..=2 {
                wave.fold_limit = fold;
                let original = wave;
                let mut reference = wave;
                reference.warp_amount = 0;
                for i in 0..=SIZE {
                    let phase = i as f32 / SIZE as f32;
                    assert_eq!(wave.sample_unwarped(phase), reference.sample(phase));
                }
                assert_eq!(wave, original);
            }
        }
    }
}

// Direct DFT is a small mathematical oracle, not another playback/build path.
#[test]
fn complex_fft_matches_direct_dft_and_round_trips_real_and_imaginary_data() {
    crate::tests::with_build_job(8, |job| {
        job.build();
        let mut signal = [Complex::default(); SIZE];
        for (i, value) in signal.iter_mut().enumerate() {
            value.re = ((i * 73 % 201) as f32 - 100.0) * 0.01;
            value.im = ((i * 31 % 113) as f32 - 56.0) * 0.01;
        }
        for inverse in [false, true] {
            let mut actual = signal;
            fft(&mut actual, &job.twiddles, inverse);
            for (k, value) in actual.iter().enumerate() {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (i, input) in signal.iter().enumerate() {
                    let angle = std::f64::consts::TAU * (i * k) as f64 / SIZE as f64
                        * if inverse { 1.0 } else { -1.0 };
                    let (sin, cos) = angle.sin_cos();
                    re += input.re as f64 * cos - input.im as f64 * sin;
                    im += input.re as f64 * sin + input.im as f64 * cos;
                }
                let scale = if inverse { 1.0 / SIZE as f64 } else { 1.0 };
                assert!((value.re as f64 - re * scale).abs() < 0.0001);
                assert!((value.im as f64 - im * scale).abs() < 0.0001);
            }
            fft(&mut actual, &job.twiddles, !inverse);
            for (actual, expected) in actual.iter().zip(signal) {
                assert!((actual.re - expected.re).abs() < 0.00001);
                assert!((actual.im - expected.im).abs() < 0.00001);
            }
        }
    });
}

fn varied_wave(count: usize, smoothing: i16, warp: i16) -> Wave {
    let mut wave = Wave::new(count);
    wave.smoothing = smoothing;
    wave.warp_amount = warp;
    wave.warp_position = 73;
    for i in 0..count {
        wave.boundaries[i] = ((i * 47 + count * 19) % 201) as i16 - 100;
        wave.curves[i] = ((i * 61 + count * 13) % 201) as i16 - 100;
    }
    wave
}

#[test]
fn worker_packed_tables_match_independent_spectral_masks_for_every_segment_count() {
    for count in 2..=MAX_SEGMENTS {
        crate::tests::with_build_job(count, |job| {
            for smoothing in [0, 1, 57, 100] {
                for warp in [-89, 0, 93] {
                    job.wave = varied_wave(count, smoothing, warp);
                    let before = FFT_COUNTS.with(Cell::get);
                    job.build();
                    let after = FFT_COUNTS.with(Cell::get);
                    assert_eq!(
                        (after.0 - before.0, after.1 - before.1),
                        if smoothing == 0 { (1, 3) } else { (2, 4) }
                    );
                    assert!(job.completed);
                    // Independently reconstruct the real source from the creative
                    // preview; each inverse below handles just one real table.
                    let mut spectrum = [Complex::default(); SIZE];
                    for (i, value) in spectrum.iter_mut().enumerate() {
                        let phase = i as f32 / SIZE as f32;
                        value.re = if smoothing == 0 {
                            job.wave.sample(phase)
                        } else {
                            let position = job.wave.unwarped_phase(phase) * SIZE as f32;
                            let index = position as usize & (SIZE - 1);
                            let fraction = position - position.floor();
                            let preview = &job.result[DEFAULT_TABLES];
                            preview[index]
                                + (preview[(index + 1) & (SIZE - 1)] - preview[index]) * fraction
                        };
                    }
                    fft(&mut spectrum, &job.twiddles, false);
                    for level in 0..DEFAULT_TABLES {
                        let mut expected = spectrum;
                        for (k, value) in expected.iter_mut().enumerate() {
                            if k.min(SIZE - k) > cutoff(level) || k == SIZE / 2 {
                                *value = Complex::default();
                            }
                        }
                        fft(&mut expected, &job.twiddles, true);
                        for (i, (actual, expected)) in
                            job.result[level].iter().zip(expected).enumerate()
                        {
                            assert!((actual - expected.re).abs() < 0.00002,
                                "count {count}, smoothing {smoothing}, warp {warp}, table {level}, sample {i}");
                            assert!(expected.im.abs() < 0.00001);
                        }
                    }
                }
            }
        });
    }
}

#[test]
fn worker_tables_remove_unsafe_harmonics_after_smoothing_and_warp() {
    crate::tests::with_build_job(32, |job| {
        for smoothing in [0, 50, 100] {
            for warp in [-100, 0, 100] {
                job.wave = varied_wave(32, smoothing, warp);
                job.build();
                for level in 0..DEFAULT_TABLES {
                    let mut spectrum = [Complex::default(); SIZE];
                    for (value, sample) in spectrum.iter_mut().zip(job.result[level]) {
                        value.re = sample;
                    }
                    fft(&mut spectrum, &job.twiddles, false);
                    let (mut pass, mut stop) = (0.0, 0.0);
                    for (k, value) in spectrum.iter().enumerate() {
                        let energy = value.re * value.re + value.im * value.im;
                        if k.min(SIZE - k) > cutoff(level) || k == SIZE / 2 {
                            stop += energy;
                        } else {
                            pass += energy;
                        }
                    }
                    assert!(pass > 0.001);
                    assert!(stop < pass * 0.00000001);
                }
            }
        }
    });
}

#[test]
fn geometry_cache_preserves_twiddles_and_smoothing_gains_and_matches_wave_mapping() {
    crate::tests::with_build_job(8, |job| {
        job.build();
        let fixed = job.twiddles;
        for count in 2..=MAX_SEGMENTS {
            for position in [0, 1, 94, 128, 254, 255] {
                for amount in [-100, -50, 50, 100] {
                    let mut wave = Wave::new(count);
                    wave.warp_position = position;
                    wave.warp_amount = amount;
                    prepare_geometry(&wave, &mut job.twiddles);
                    for (a, b) in job.twiddles[..POSITION_CACHE].iter().zip(fixed) {
                        assert_eq!((a.re, a.im), (b.re, b.im));
                    }
                    let mut cursor = 0;
                    for i in 0..SIZE {
                        let phase = i as f32 / SIZE as f32;
                        let expected = wave.segment_at(phase);
                        let actual = cached_segment_at(&wave, &job.twiddles, phase, &mut cursor);
                        assert_eq!(actual.0, expected.0);
                        assert!((actual.1 - expected.1).abs() < 0.000001);
                    }
                }
            }
        }
    });
}

#[test]
fn published_editor_preserves_boundaries_seam_and_pitch_independence() {
    for count in 2..=MAX_SEGMENTS {
        let mut engine = Engine::new(count, 48000);
        let mut memory = [Block::ZERO; BLOCKS];
        for smoothing in [0, 1, 50, 100] {
            for warp in [-89, 0, 93] {
                let wave = varied_wave(count, smoothing, warp);
                finish_build(&mut engine, &mut memory, wave);
                for boundary in 0..=count {
                    let sample =
                        engine.editor_phase_sample(&memory, wave.boundary_position(boundary));
                    assert!((sample - wave.boundary_level(boundary)).abs() < 0.0005);
                }
                assert_eq!(
                    engine.editor_phase_sample(&memory, 0.0),
                    engine.editor_phase_sample(&memory, 1.0)
                );
                let before = memory;
                let mut voice = Oscillator::new();
                for frequency in [20.0, 440.0, 2800.0, 19000.0] {
                    voice.prepare(frequency, 48000.0, 0.5);
                    for frame in 0..64 {
                        assert!(voice.tick_prepared(&engine, &memory, frame).is_finite());
                    }
                    for i in 0..SIZE {
                        let phase = i as f32 / SIZE as f32;
                        if smoothing == 0 {
                            assert_eq!(
                                engine.editor_phase_sample(&memory, phase),
                                wave.sample(phase)
                            );
                        }
                    }
                }
                for (a, b) in memory.iter().zip(before) {
                    for (a, b) in a.data.iter().zip(b.data) {
                        assert_eq!((a.re, a.im), (b.re, b.im));
                    }
                }
            }
        }
    }
}

#[test]
fn smoothing_keeps_changing_through_the_upper_control_range() {
    for count in [2, 8, 32] {
        crate::tests::with_build_job(count, |job| {
            let mut previous = [0.0; SIZE];
            for smoothing in [30, 40, 50, 60, 70, 80, 90, 100] {
                job.wave = varied_wave(count, smoothing, 0);
                job.build();
                let difference: f32 = previous
                    .iter()
                    .zip(job.result[DEFAULT_TABLES])
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum();
                if smoothing > 30 {
                    assert!(difference / SIZE as f32 > 0.000001);
                }
                previous = job.result[DEFAULT_TABLES];
            }
        });
    }
}

#[test]
fn six_table_selection_preserves_safe_partials_and_is_continuous_at_transitions() {
    let layout = TableLayout::new(DEFAULT_TABLES);
    assert_eq!(&layout.cutoffs[..DEFAULT_TABLES], &[127, 32, 8, 4, 2, 1]);
    let gain = |increment, harmonic| {
        let (level, blend) = playback_levels(increment);
        if harmonic > cutoff(level) {
            0.0
        } else if level + 1 == DEFAULT_TABLES || harmonic <= cutoff(level + 1) {
            1.0
        } else {
            1.0 - blend
        }
    };
    for i in 1..=45000 {
        let increment = i as f32 / 100000.0;
        let (level, blend) = playback_levels(increment);
        assert!(level < DEFAULT_TABLES && (0.0..=1.0).contains(&blend));
        assert!(cutoff(level) as f32 * increment <= 0.490001);
        for harmonic in 1..SIZE / 2 {
            let frequency = increment * harmonic as f32;
            if frequency <= 0.12 {
                assert!(gain(increment, harmonic) > 0.99999);
            }
            if frequency >= 0.49 {
                assert_eq!(gain(increment, harmonic), 0.0);
            }
        }
    }
    let mut engine = Engine::new(8, 48000);
    let mut memory = [Block::ZERO; BLOCKS];
    finish_build(&mut engine, &mut memory, varied_wave(8, 50, 20));
    for level in 0..DEFAULT_TABLES - 1 {
        let threshold = 0.49 / cutoff(level) as f32;
        let (lo, lb) = playback_levels(threshold - 0.0000001);
        let (hi, hb) = playback_levels(threshold + 0.0000001);
        for i in 0..SIZE {
            let phase = (i as u32) << 24;
            let before = engine.voice_sample(&memory, engine.active, phase, lo, lb);
            let after = engine.voice_sample(&memory, engine.active, phase, hi, hb);
            assert!((before - after).abs() < 0.001);
        }
    }
}

#[test]
fn pitch_slew_follows_exponential_model_monotonically_and_settles_exactly() {
    for rate in [32000.0, 44100.0, 48000.0, 96000.0] {
        for (start, target) in [
            (20.0, 20000.0),
            (20000.0, 20.0),
            (440.0, 441.0),
            (440.0, 440.0001),
        ] {
            let mut voice = Oscillator::new();
            voice.tune_when_silent(start, rate);
            voice.prepare(target, rate, 0.5);
            let alpha = voice.slew as f64;
            let distance = voice.target_step.abs_diff(voice.phase_step) as f64;
            let mut expected = voice.phase_step as f64;
            let mut sum = 0.0;
            for _ in 0..5000 {
                let before = voice.phase_step;
                expected += alpha * (voice.target_step as f64 - expected);
                voice.advance_pitch_step();
                assert!(
                    (before.min(voice.target_step)..=before.max(voice.target_step))
                        .contains(&voice.phase_step)
                );
                let tolerance = 3.0 / alpha
                    + 8.0
                    + if voice.pair_pending {
                        0.5 * alpha * alpha * distance
                    } else {
                        0.0
                    };
                assert!((voice.phase_step as f64 - expected).abs() <= tolerance);
                sum += voice.phase_step as f64 - expected;
            }
            assert_eq!(voice.phase_step, voice.target_step);
            assert!(!voice.pitch_moving && !voice.pair_pending);
            assert!(
                sum.abs() <= distance * alpha / (2.0 * (2.0 - alpha)) + 0.001 * PHASE_SCALE as f64
            );
            let hz = voice.phase_step as f64 * rate as f64 / PHASE_SCALE as f64;
            assert!(1200.0 * (hz / (target.min(rate * 0.45)) as f64).log2().abs() < 0.002);
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
    finish_build(&mut engine, &mut memory, wave);
    let mut voice = Oscillator::new();
    voice.tune_when_silent(frequency, 48000.0);
    voice.level = 1.0;
    voice.prepare(frequency, 48000.0, 1.0);
    let (mut raw_cos, mut raw_sin, mut clean_cos, mut clean_sin) = (0.0, 0.0, 0.0, 0.0);
    let (mut fundamental_cos, mut fundamental_sin) = (0.0, 0.0);
    for i in 0..FRAMES {
        let raw = wave.sample(voice.phase as f32 / 4_294_967_296.0);
        let clean = voice.tick_prepared(&engine, &memory, 0) / 5.0;
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
fn smoothing_gains_remain_fixed_and_preview_varies_linearly_with_amount() {
    for count in 2..=MAX_SEGMENTS {
        crate::tests::with_build_job(count, |job| {
            job.wave = varied_wave(count, 1, 0);
            job.build();
            let first = job.result[DEFAULT_TABLES];
            let constants = job.twiddles;
            for harmonic in 0..=SIZE / 2 {
                let ratio = harmonic as f32 / 4.0;
                let expected = 1.0 / (1.0 + ratio.powi(4));
                let slot = constants[SIZE / 2 + harmonic / 2];
                assert_eq!(expected, if harmonic & 1 == 0 { slot.re } else { slot.im });
            }
            job.wave.smoothing = 100;
            job.build();
            let last = job.result[DEFAULT_TABLES];
            job.wave.smoothing = 50;
            job.build();
            for (i, actual) in job.result[DEFAULT_TABLES].iter().enumerate() {
                let expected = first[i] + (last[i] - first[i]) * (49.0 / 99.0);
                assert!((actual - expected).abs() < 0.00001);
            }
            for (a, b) in constants[..POSITION_CACHE].iter().zip(job.twiddles) {
                assert_eq!((a.re, a.im), (b.re, b.im));
            }
        });
    }
}
