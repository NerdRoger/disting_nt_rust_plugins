use super::*;

fn run_gate(midi: &mut MidiState, frames: usize) {
    for _ in 0..frames {
        midi.begin_block();
        for group in &mut midi.groups {
            group.next_gain(midi.attack_step, midi.gate_step);
        }
    }
}

#[test]
fn allocation_steals_oldest_and_old_note_off_cannot_release_its_replacement() {
    let mut midi = MidiState::new(48000);
    midi.set_channel(1);
    let enabled = [true; GROUP_COUNT];
    for note in [60, 64, 67, 71, 72, 73, 75, 77, 80, 83] {
        midi.message(0x90, note, 100, &enabled);
    }
    run_gate(&mut midi, 250);
    midi.message(0x90, 74, 100, &enabled);
    assert_eq!(midi.groups[0].note, 74);
    assert_eq!(midi.groups[0].playing_note, 60);
    assert!(midi.groups[0].switching);
    midi.message(0x80, 60, 0, &enabled);
    assert!(midi.groups[0].held);
    run_gate(&mut midi, 500);
    assert_eq!(midi.groups[0].playing_note, 74);
    assert_eq!(midi.groups[0].gain, 1.0);
    midi.message(0x90, 76, 100, &enabled);
    assert_eq!(midi.groups[1].note, 76);
    midi.message(0x80, 67, 0, &enabled);
    midi.message(0x90, 79, 100, &enabled);
    assert_eq!(midi.groups[2].note, 79);
    assert_eq!(midi.groups[3].note, 71);
}

#[test]
fn channel_filter_disabled_outputs_and_velocity_zero_note_off() {
    let mut midi = MidiState::new(48000);
    let mut enabled = [false; GROUP_COUNT];
    enabled[1] = true;
    enabled[3] = true;
    midi.message(0x90, 60, 100, &enabled);
    assert!(midi.groups.iter().all(|v| !v.held));
    midi.set_channel(3); // channel 2
    midi.message(0x90, 60, 100, &enabled);
    assert!(midi.groups.iter().all(|v| !v.held));
    midi.message(0x91, 60, 100, &enabled);
    midi.message(0x91, 64, 100, &enabled);
    assert_eq!(midi.groups[1].note, 60);
    assert_eq!(midi.groups[3].note, 64);
    assert_eq!(midi.groups[0], NoteGroup::new());
    midi.message(0x91, 67, 100, &enabled);
    assert_eq!(midi.groups[1].note, 67);
    midi.message(0x90, 67, 0, &enabled);
    assert!(midi.groups[1].held);
    midi.message(0x91, 67, 0, &enabled);
    assert!(!midi.groups[1].held);
    midi.set_channel(0);
    assert!(midi.groups.iter().all(|v| *v == NoteGroup::new()));
}

#[test]
fn gates_fade_and_pending_note_off_does_not_restart_a_stolen_group() {
    let mut midi = MidiState::new(48000);
    midi.set_channel(1);
    let mut enabled = [false; GROUP_COUNT];
    enabled[0] = true;
    midi.message(0x90, 60, 127, &enabled);
    run_gate(&mut midi, 120);
    assert!((midi.groups[0].gain - 0.5).abs() < 0.001);
    run_gate(&mut midi, 130);
    assert_eq!(midi.groups[0].gain, 1.0);
    midi.message(0x90, 67, 127, &enabled);
    midi.message(0x80, 67, 0, &enabled);
    run_gate(&mut midi, 500);
    assert!(!midi.groups[0].sounding());
    assert_eq!(midi.groups[0].gain, 0.0);
    midi.message(0x90, 72, 127, &enabled);
    midi.message(0x90, 72, 127, &enabled);
    assert_eq!(midi.groups.iter().filter(|v| v.held).count(), 1);
    midi.message(0xb0, 123, 0, &enabled);
    run_gate(&mut midi, 250);
    assert!(!midi.groups[0].sounding());
    midi.message(0x90, 74, 127, &enabled);
    run_gate(&mut midi, 120);
    midi.message(0xb0, 120, 0, &enabled);
    assert_eq!(midi.groups[0], NoteGroup::new());
}

#[test]
fn all_channels_track_note_off_by_channel_and_group_order_stays_bounded() {
    let mut midi = MidiState::new(48000);
    midi.set_channel(1);
    let enabled = [true; GROUP_COUNT];
    midi.message(0x90, 60, 127, &enabled);
    midi.message(0x91, 60, 127, &enabled);
    midi.message(0x80, 60, 0, &enabled);
    assert!(!midi.groups[0].held && midi.groups[1].held);
    for i in 0..2000 {
        midi.message(0x90, (i % 128) as u8, 127, &enabled);
        if i % 3 == 0 {
            midi.message(0x80, ((i + 127) % 128) as u8, 0, &enabled);
        }
        for group in &midi.groups {
            if group.held {
                assert!(group.order < GROUP_COUNT as u8);
            }
        }
        for (a, group) in midi.groups.iter().enumerate() {
            if group.held {
                assert!(midi
                    .groups
                    .iter()
                    .skip(a + 1)
                    .all(|other| !other.held || other.order != group.order));
            }
        }
    }
    midi.sync_enabled(&[false; GROUP_COUNT]);
    assert!(midi.groups.iter().all(|v| *v == NoteGroup::new()));
}

#[test]
fn every_configured_pool_uses_idle_groups_then_steals_only_its_oldest_note() {
    for count in 1..=GROUP_COUNT {
        let mut midi = MidiState::new(48000);
        midi.set_channel(1);
        let mut enabled = [false; GROUP_COUNT];
        for on in enabled.iter_mut().take(count) {
            *on = true;
        }
        for note in 0..count {
            midi.message(0x90, 60 + note as u8, 100, &enabled);
        }
        run_gate(&mut midi, 250);
        assert_eq!(midi.groups.iter().filter(|v| v.held).count(), count);
        midi.message(0x90, 84, 100, &enabled);
        assert_eq!(midi.groups[0].note, 84);
        midi.message(0x80, 60, 0, &enabled);
        assert!(midi.groups[0].held);
        run_gate(&mut midi, 500);
        assert_eq!(midi.groups[0].playing_note, 84);
        assert!(midi
            .groups
            .iter()
            .skip(count)
            .all(|v| *v == NoteGroup::new()));
    }
}

#[test]
fn pitch_bend_decodes_all_fourteen_bits_and_respects_channel_filter_and_reset() {
    let mut midi = MidiState::new(48000);
    let enabled = [true; GROUP_COUNT];
    midi.message(0xe0, 0, 0, &enabled);
    assert_eq!(midi.pitch_bends, [0.0; 16]); // MIDI Off.
    midi.set_channel(3); // Channel 2 only.
    midi.message(0xe0, 0, 0, &enabled);
    assert_eq!(midi.pitch_bends, [0.0; 16]);
    for value in [0u16, 8191, 8192, 8193, 12288, 16383] {
        midi.message(0xe1, (value & 127) as u8, (value >> 7) as u8, &enabled);
        let expected = (value as i32 - 8192) as f32 / (8192.0 * 6.0);
        assert_eq!(midi.pitch_bends[1], expected);
        assert_eq!(midi.pitch_bends[0], 0.0);
    }
    let before = midi.pitch_bends;
    midi.message(0xe1, 128, 64, &enabled);
    midi.message(0xe1, 0, 128, &enabled);
    assert_eq!(midi.pitch_bends, before);
    midi.set_channel(3); // An unchanged setting must preserve wheel position.
    assert_eq!(midi.pitch_bends, before);
    midi.set_channel(1);
    assert_eq!(midi.pitch_bends, [0.0; 16]);
}

#[test]
fn bend_follows_the_playing_channel_until_a_stolen_group_finishes_fading() {
    let mut midi = MidiState::new(48000);
    midi.set_channel(1);
    let mut enabled = [false; GROUP_COUNT];
    enabled[0] = true;
    midi.message(0xe0, 0, 0, &enabled);
    midi.message(0xe1, 0, 96, &enabled);
    midi.message(0x90, 60, 100, &enabled);
    run_gate(&mut midi, 250);
    assert_eq!(midi.groups[0].bend_octaves(&midi.pitch_bends), -2.0 / 12.0);
    midi.message(0x91, 72, 100, &enabled);
    assert_eq!(midi.groups[0].pitch(), 60);
    assert_eq!(midi.groups[0].bend_octaves(&midi.pitch_bends), -2.0 / 12.0);
    run_gate(&mut midi, 500);
    assert_eq!(midi.groups[0].pitch(), 72);
    assert_eq!(midi.groups[0].bend_octaves(&midi.pitch_bends), 1.0 / 12.0);
    midi.message(0x81, 72, 0, &enabled);
    midi.message(0xe1, 0, 0, &enabled);
    assert!(midi.groups[0].sounding());
    assert_eq!(midi.groups[0].bend_octaves(&midi.pitch_bends), -2.0 / 12.0);
}

#[test]
fn sparse_active_pool_excludes_off_groups_and_steals_only_among_five_enabled_groups() {
    let mut midi = MidiState::new(48000);
    midi.set_channel(1);
    let mut enabled = [false; GROUP_COUNT];
    for group in [1, 2, 3, 5, 7] {
        enabled[group] = true;
    }
    for note in 60..65 {
        midi.message(0x90, note, 100, &enabled);
    }
    run_gate(&mut midi, 250);
    for (group, &on) in midi.groups.iter().zip(&enabled) {
        assert_eq!(group.held, on);
    }
    midi.message(0x90, 72, 100, &enabled);
    assert_eq!(midi.groups[1].note, 72);
    midi.message(0x80, 60, 0, &enabled);
    assert!(midi.groups[1].held);
    enabled[1] = false;
    midi.release_disabled(&enabled);
    assert!(!midi.groups[1].held);
    run_gate(&mut midi, 500);
    assert!(!midi.groups[1].sounding());
    for note in 80..90 {
        midi.message(0x90, note, 100, &enabled);
    }
    assert!(!midi.groups[1].held);
    assert_eq!(midi.groups.iter().filter(|group| group.held).count(), 4);
    midi.release_disabled(&[false; GROUP_COUNT]);
    midi.message(0x90, 100, 100, &[false; GROUP_COUNT]);
    run_gate(&mut midi, 250);
    assert!(midi.groups.iter().all(|v| !v.sounding()));
}

#[test]
fn default_attack_is_five_ms_and_release_is_five_hundred_ms() {
    let mut midi = MidiState::new(48000);
    midi.set_channel(1);
    let enabled = [true; GROUP_COUNT];
    midi.message(0x90, 60, 100, &enabled);
    run_gate(&mut midi, 120);
    assert!((midi.groups[0].gain - 0.5).abs() < 0.0001);
    run_gate(&mut midi, 130);
    midi.message(0x80, 60, 0, &enabled);
    run_gate(&mut midi, 12000);
    assert!((midi.groups[0].gain - 0.5).abs() < 0.001);
    run_gate(&mut midi, 12000);
    assert_eq!(midi.groups[0].gate_state(), GateState::Silent);
}

#[test]
fn attack_and_release_are_independent_and_early_note_off_starts_from_current_gain() {
    let mut midi = MidiState::new(1000);
    midi.set_channel(1);
    midi.set_envelope(100, 1000);
    let enabled = [true; GROUP_COUNT];
    midi.message(0x90, 60, 100, &enabled);
    run_gate(&mut midi, 50);
    assert!((midi.groups[0].gain - 0.5).abs() < 0.00001);
    midi.message(0x80, 60, 0, &enabled);
    let release_start = midi.groups[0].gain;
    midi.message(0x90, 64, 100, &enabled);
    run_gate(&mut midi, 100);
    assert!((midi.groups[0].gain - release_start * 0.9).abs() < 0.00001);
    assert!((midi.groups[1].gain - 1.0).abs() < 0.00001);
    run_gate(&mut midi, 900);
    assert_eq!(midi.groups[0].gain, 0.0);
    assert_eq!(midi.groups[1].gain, 1.0);
}

#[test]
fn stealing_keeps_its_short_fade_and_replacement_uses_the_selected_attack() {
    let mut midi = MidiState::new(1000);
    midi.set_channel(1);
    midi.set_envelope(100, 1000);
    let mut enabled = [false; GROUP_COUNT];
    enabled[0] = true;
    midi.message(0x90, 60, 100, &enabled);
    run_gate(&mut midi, 110);
    midi.message(0x90, 67, 100, &enabled);
    run_gate(&mut midi, 6); // Allow the existing float fade one rounding sample.
    assert_eq!(midi.groups[0].gain, 0.0);
    midi.begin_block();
    assert_eq!(midi.groups[0].pitch(), 67);
    run_gate(&mut midi, 50);
    assert!((midi.groups[0].gain - 0.5).abs() < 0.00001);
    midi.message(0xb0, 120, 0, &enabled);
    assert_eq!(midi.groups[0], NoteGroup::new());
}

#[test]
fn envelope_ranges_clamp_and_cached_settings_do_not_restart_envelopes() {
    let mut midi = MidiState::new(48000);
    midi.set_channel(1);
    midi.set_envelope(-1, 2000);
    assert_eq!(
        (midi.attack_ms, midi.release_ms, midi.release_frames),
        (1, 1000, 48000)
    );
    midi.message(0x90, 60, 100, &[true; GROUP_COUNT]);
    run_gate(&mut midi, 24);
    let gain = midi.groups[0].gain;
    midi.set_envelope(1, 1000);
    assert_eq!(midi.groups[0].gain, gain);
    run_gate(&mut midi, 30);
    midi.set_envelope(500, 1);
    midi.message(0x90, 60, 0, &[true; GROUP_COUNT]);
    run_gate(&mut midi, 48);
    assert!(!midi.groups[0].sounding());
}
