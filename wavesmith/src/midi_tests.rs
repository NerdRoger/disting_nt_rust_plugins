use super::*;

fn run_gate(midi: &mut MidiState, frames: usize) {
    for _ in 0..frames {
        midi.begin_block();
        for voice in &mut midi.voices {
            voice.next_gain(midi.gate_step);
        }
    }
}

#[test]
fn allocation_steals_oldest_and_old_note_off_cannot_release_its_replacement() {
    let mut midi = MidiState::new(48000);
    midi.set_channel(1);
    let enabled = [true; VOICE_COUNT];
    for note in [60, 64, 67, 71, 72, 73, 75, 77, 80, 83] {
        midi.message(0x90, note, 100, &enabled);
    }
    run_gate(&mut midi, 250);
    midi.message(0x90, 74, 100, &enabled);
    assert_eq!(midi.voices[0].note, 74);
    assert_eq!(midi.voices[0].playing_note, 60);
    assert!(midi.voices[0].switching);
    midi.message(0x80, 60, 0, &enabled);
    assert!(midi.voices[0].held);
    run_gate(&mut midi, 500);
    assert_eq!(midi.voices[0].playing_note, 74);
    assert_eq!(midi.voices[0].gain, 1.0);
    midi.message(0x90, 76, 100, &enabled);
    assert_eq!(midi.voices[1].note, 76);
    midi.message(0x80, 67, 0, &enabled);
    midi.message(0x90, 79, 100, &enabled);
    assert_eq!(midi.voices[2].note, 79);
    assert_eq!(midi.voices[3].note, 71);
}

#[test]
fn channel_filter_disabled_outputs_and_velocity_zero_note_off() {
    let mut midi = MidiState::new(48000);
    let mut enabled = [false; VOICE_COUNT];
    enabled[1] = true;
    enabled[3] = true;
    midi.message(0x90, 60, 100, &enabled);
    assert!(midi.voices.iter().all(|v| !v.held));
    midi.set_channel(3); // channel 2
    midi.message(0x90, 60, 100, &enabled);
    assert!(midi.voices.iter().all(|v| !v.held));
    midi.message(0x91, 60, 100, &enabled);
    midi.message(0x91, 64, 100, &enabled);
    assert_eq!(midi.voices[1].note, 60);
    assert_eq!(midi.voices[3].note, 64);
    assert_eq!(midi.voices[0], NoteVoice::new());
    midi.message(0x91, 67, 100, &enabled);
    assert_eq!(midi.voices[1].note, 67);
    midi.message(0x90, 67, 0, &enabled);
    assert!(midi.voices[1].held);
    midi.message(0x91, 67, 0, &enabled);
    assert!(!midi.voices[1].held);
    midi.set_channel(0);
    assert!(midi.voices.iter().all(|v| *v == NoteVoice::new()));
}

#[test]
fn gates_fade_and_pending_note_off_does_not_restart_a_stolen_voice() {
    let mut midi = MidiState::new(48000);
    midi.set_channel(1);
    let mut enabled = [false; VOICE_COUNT];
    enabled[0] = true;
    midi.message(0x90, 60, 127, &enabled);
    run_gate(&mut midi, 120);
    assert!((midi.voices[0].gain - 0.5).abs() < 0.001);
    run_gate(&mut midi, 130);
    assert_eq!(midi.voices[0].gain, 1.0);
    midi.message(0x90, 67, 127, &enabled);
    midi.message(0x80, 67, 0, &enabled);
    run_gate(&mut midi, 500);
    assert!(!midi.voices[0].sounding());
    assert_eq!(midi.voices[0].gain, 0.0);
    midi.message(0x90, 72, 127, &enabled);
    midi.message(0x90, 72, 127, &enabled);
    assert_eq!(midi.voices.iter().filter(|v| v.held).count(), 1);
    midi.message(0xb0, 123, 0, &enabled);
    run_gate(&mut midi, 250);
    assert!(!midi.voices[0].sounding());
    midi.message(0x90, 74, 127, &enabled);
    run_gate(&mut midi, 120);
    midi.message(0xb0, 120, 0, &enabled);
    assert_eq!(midi.voices[0], NoteVoice::new());
}

#[test]
fn all_channels_track_note_off_by_channel_and_voice_order_stays_bounded() {
    let mut midi = MidiState::new(48000);
    midi.set_channel(1);
    let enabled = [true; VOICE_COUNT];
    midi.message(0x90, 60, 127, &enabled);
    midi.message(0x91, 60, 127, &enabled);
    midi.message(0x80, 60, 0, &enabled);
    assert!(!midi.voices[0].held && midi.voices[1].held);
    for i in 0..2000 {
        midi.message(0x90, (i % 128) as u8, 127, &enabled);
        if i % 3 == 0 {
            midi.message(0x80, ((i + 127) % 128) as u8, 0, &enabled);
        }
        for voice in &midi.voices {
            if voice.held {
                assert!(voice.order < VOICE_COUNT as u8);
            }
        }
        for (a, voice) in midi.voices.iter().enumerate() {
            if voice.held {
                assert!(midi
                    .voices
                    .iter()
                    .skip(a + 1)
                    .all(|other| !other.held || other.order != voice.order));
            }
        }
    }
    midi.sync_enabled(&[false; VOICE_COUNT]);
    assert!(midi.voices.iter().all(|v| *v == NoteVoice::new()));
}

#[test]
fn every_configured_pool_uses_idle_voices_then_steals_only_its_oldest_note() {
    for count in 1..=VOICE_COUNT {
        let mut midi = MidiState::new(48000);
        midi.set_channel(1);
        let mut enabled = [false; VOICE_COUNT];
        for on in enabled.iter_mut().take(count) {
            *on = true;
        }
        for note in 0..count {
            midi.message(0x90, 60 + note as u8, 100, &enabled);
        }
        run_gate(&mut midi, 250);
        assert_eq!(midi.voices.iter().filter(|v| v.held).count(), count);
        midi.message(0x90, 84, 100, &enabled);
        assert_eq!(midi.voices[0].note, 84);
        midi.message(0x80, 60, 0, &enabled);
        assert!(midi.voices[0].held);
        run_gate(&mut midi, 500);
        assert_eq!(midi.voices[0].playing_note, 84);
        assert!(midi
            .voices
            .iter()
            .skip(count)
            .all(|v| *v == NoteVoice::new()));
    }
}
