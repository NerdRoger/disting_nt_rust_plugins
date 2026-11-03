//! Fixed-size MIDI allocator and short click-suppression gates. No audio buffers.
#![forbid(unsafe_code)]
use super::VOICE_COUNT;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteVoice {
    note: u8,
    channel: u8,
    order: u8,
    held: bool,
    playing_note: u8,
    playing_channel: u8,
    switching: bool,
    retune: bool,
    gain: f32,
}

impl NoteVoice {
    const fn new() -> Self {
        Self {
            note: 0,
            channel: 0,
            order: 0,
            held: false,
            playing_note: 0,
            playing_channel: 0,
            switching: false,
            retune: false,
            gain: 0.0,
        }
    }
    pub fn sounding(&self) -> bool {
        self.held || self.gain > 0.0
    }
    pub fn pitch(&self) -> u8 {
        self.playing_note
    }
    pub fn bend_octaves(&self, pitch_bends: &[f32; 16]) -> f32 {
        pitch_bends[(self.playing_channel & 0x0f) as usize]
    }
    pub fn take_retune(&mut self) -> bool {
        core::mem::replace(&mut self.retune, false)
    }
    pub fn gate_is_steady(&self) -> bool {
        self.held && !self.switching && self.gain == 1.0
    }
    pub fn next_gain(&mut self, step: f32) -> f32 {
        self.gain = if self.held && !self.switching {
            (self.gain + step).min(1.0)
        } else {
            (self.gain - step).max(0.0)
        };
        self.gain
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MidiState {
    pub voices: [NoteVoice; VOICE_COUNT],
    pub gate_step: f32,
    // Cached octave offsets per MIDI channel; center is zero. Retain bend
    // before Note On and while the output is disabled, without retuning CV.
    pub pitch_bends: [f32; 16],
    channel_setting: u8,
}

impl MidiState {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            voices: [NoteVoice::new(); VOICE_COUNT],
            gate_step: 1.0 / (sample_rate / 200).max(1) as f32,
            channel_setting: 0,
            pitch_bends: [0.0; 16],
        }
    }
    pub fn set_channel(&mut self, setting: u8) -> bool {
        if setting == self.channel_setting {
            return false;
        }
        self.channel_setting = setting;
        self.pitch_bends = [0.0; 16];
        self.voices = [NoteVoice::new(); VOICE_COUNT];
        true
    }
    pub fn sync_enabled(&mut self, enabled: &[bool; VOICE_COUNT]) {
        for (index, &enabled) in enabled.iter().enumerate() {
            if !enabled {
                self.forget(index);
            }
        }
    }
    pub fn forget(&mut self, index: usize) {
        self.release(index);
        if let Some(voice) = self.voices.get_mut(index) {
            *voice = NoteVoice::new();
        }
    }
    fn release(&mut self, index: usize) {
        let Some(voice) = self.voices.get(index) else {
            return;
        };
        if !voice.held {
            return;
        }
        let order = voice.order;
        for (i, voice) in self.voices.iter_mut().enumerate() {
            if i == index {
                voice.held = false;
            } else if voice.held && voice.order > order {
                voice.order -= 1;
            }
        }
    }
    pub fn begin_block(&mut self) {
        for voice in &mut self.voices {
            if voice.switching && voice.gain == 0.0 {
                if voice.held {
                    voice.playing_note = voice.note;
                    voice.playing_channel = voice.channel;
                    voice.retune = true;
                }
                voice.switching = false;
            }
        }
    }
    pub fn message(&mut self, status: u8, data1: u8, data2: u8, enabled: &[bool; VOICE_COUNT]) {
        if self.channel_setting == 0 || data1 > 127 || data2 > 127 {
            return;
        }
        let channel = status & 0x0f;
        if self.channel_setting != 1 && self.channel_setting != channel + 2 {
            return;
        }
        self.sync_enabled(enabled);
        match status & 0xf0 {
            0xe0 => {
                let value = ((data2 as i32) << 7) | data1 as i32;
                // 14-bit unsigned bend: 8192 is neutral. Fixed +/-2 semitones;
                // positive full scale is one MIDI unit short of +2 semitones.
                self.pitch_bends[channel as usize] = (value - 8192) as f32 / (8192.0 * 6.0);
            }
            0x90 if data2 != 0 => self.note_on(channel, data1, enabled),
            0x80 | 0x90 => {
                if let Some(index) = self
                    .voices
                    .iter()
                    .position(|voice| voice.held && voice.note == data1 && voice.channel == channel)
                {
                    self.release(index);
                }
            }
            0xb0 if data1 == 123 || data1 == 120 => {
                for index in 0..VOICE_COUNT {
                    if self
                        .voices
                        .get(index)
                        .is_some_and(|voice| voice.channel == channel)
                    {
                        if data1 == 120 {
                            self.forget(index);
                        } else {
                            self.release(index);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn note_on(&mut self, channel: u8, note: u8, enabled: &[bool; VOICE_COUNT]) {
        // Retrigger an already-held key; otherwise prefer fully idle voices,
        // then released voices, then steal the oldest held note (order zero).
        let index = self
            .voices
            .iter()
            .position(|v| v.held && v.channel == channel && v.note == note)
            .or_else(|| {
                self.voices
                    .iter()
                    .zip(enabled)
                    .position(|(v, &on)| on && !v.held && v.gain == 0.0 && !v.switching)
            })
            .or_else(|| {
                self.voices
                    .iter()
                    .zip(enabled)
                    .position(|(v, &on)| on && !v.held)
            })
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.held)
                    .min_by_key(|(_, v)| v.order)
                    .map(|(i, _)| i)
            });
        let Some(index) = index else {
            return;
        };
        self.release(index);
        let newest = self.voices.iter().filter(|v| v.held).count() as u8;
        if let Some(voice) = self.voices.get_mut(index) {
            voice.note = note;
            voice.channel = channel;
            voice.order = newest;
            voice.held = true;
            voice.switching = voice.gain > 0.0;
            if !voice.switching {
                voice.playing_note = note;
                voice.playing_channel = channel;
                voice.retune = true;
            }
        }
    }
}

#[cfg(test)]
#[path = "midi_tests.rs"]
mod tests;
