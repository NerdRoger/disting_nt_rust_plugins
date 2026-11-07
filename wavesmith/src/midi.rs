//! Fixed-size MIDI allocator with attack/release envelopes and short stealing fades.
#![forbid(unsafe_code)]
pub const GROUP_COUNT: usize = 10;
pub const NO_GROUP: u8 = GROUP_COUNT as u8;
pub const ATTACK_DEFAULT: i16 = 5;
pub const RELEASE_DEFAULT: i16 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateState {
    Silent,
    Steady,
    Ramping,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteGroup {
    note: u8,
    channel: u8,
    order: u8,
    held: bool,
    playing_note: u8,
    playing_channel: u8,
    switching: bool,
    retune: bool,
    gain: f32,
    release_step: f32,
    release_remaining: u32,
}

impl NoteGroup {
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
            release_step: 0.0,
            release_remaining: 0,
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
    pub fn gate_state(&self) -> GateState {
        if self.gate_is_steady() {
            GateState::Steady
        } else if self.sounding() {
            GateState::Ramping
        } else {
            GateState::Silent
        }
    }
    pub fn next_gain(&mut self, attack_step: f32, steal_step: f32) -> f32 {
        self.gain = if self.switching {
            (self.gain - steal_step).max(0.0)
        } else if self.held {
            (self.gain + attack_step).min(1.0)
        } else if self.release_remaining > 1 {
            self.release_remaining -= 1;
            (self.gain - self.release_step).max(0.0)
        } else {
            self.release_remaining = 0;
            0.0
        };
        self.gain
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MidiState {
    pub groups: [NoteGroup; GROUP_COUNT],
    pub gate_step: f32,
    pub attack_step: f32,
    samples_per_ms: f32,
    attack_ms: i16,
    release_ms: i16,
    release_frames: u32,
    // Cached octave offsets per MIDI channel; center is zero. Retain bend
    // before Note On and while the output is disabled, without retuning CV.
    pub pitch_bends: [f32; 16],
    channel_setting: u8,
}

impl MidiState {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            groups: [NoteGroup::new(); GROUP_COUNT],
            gate_step: 1.0 / (sample_rate / 200).max(1) as f32,
            attack_step: 1.0 / (sample_rate / 200).max(1) as f32,
            samples_per_ms: sample_rate as f32 * 0.001,
            attack_ms: ATTACK_DEFAULT,
            release_ms: RELEASE_DEFAULT,
            release_frames: (sample_rate as u64 * RELEASE_DEFAULT as u64 / 1000).max(1) as u32,
            channel_setting: 0,
            pitch_bends: [0.0; 16],
        }
    }
    pub fn set_envelope(&mut self, attack_ms: i16, release_ms: i16) {
        let attack_ms = attack_ms.clamp(1, 500);
        let release_ms = release_ms.clamp(1, 1000);
        if attack_ms != self.attack_ms {
            self.attack_ms = attack_ms;
            self.attack_step = 1.0 / (self.samples_per_ms * attack_ms as f32).max(1.0);
        }
        if release_ms != self.release_ms {
            self.release_ms = release_ms;
            self.release_frames = (self.samples_per_ms * release_ms as f32 + 0.5).max(1.0) as u32;
        }
    }
    pub fn set_channel(&mut self, setting: u8) -> bool {
        if setting == self.channel_setting {
            return false;
        }
        self.channel_setting = setting;
        self.pitch_bends = [0.0; 16];
        self.groups = [NoteGroup::new(); GROUP_COUNT];
        true
    }
    pub fn sync_enabled(&mut self, enabled: &[bool; GROUP_COUNT]) {
        for (index, &enabled) in enabled.iter().enumerate() {
            if !enabled {
                self.forget(index);
            }
        }
    }
    /// Remove disabled groups from allocation immediately while allowing their
    /// existing note to finish the normal short release gate.
    pub fn release_disabled(&mut self, enabled: &[bool; GROUP_COUNT]) {
        for (index, &on) in enabled.iter().enumerate() {
            if !on {
                self.release(index);
            }
        }
    }
    pub fn forget(&mut self, index: usize) {
        self.release(index);
        if let Some(group) = self.groups.get_mut(index) {
            *group = NoteGroup::new();
        }
    }
    fn release(&mut self, index: usize) {
        let Some(group) = self.groups.get(index) else {
            return;
        };
        if !group.held {
            return;
        }
        let order = group.order;
        for (i, group) in self.groups.iter_mut().enumerate() {
            if i == index {
                group.held = false;
                group.release_remaining = self.release_frames;
                group.release_step = group.gain / self.release_frames as f32;
            } else if group.held && group.order > order {
                group.order -= 1;
            }
        }
    }
    pub fn begin_block(&mut self) {
        for group in &mut self.groups {
            if group.switching && group.gain == 0.0 {
                if group.held {
                    group.playing_note = group.note;
                    group.playing_channel = group.channel;
                    group.retune = true;
                }
                group.switching = false;
            }
        }
    }
    pub fn message(&mut self, status: u8, data1: u8, data2: u8, enabled: &[bool; GROUP_COUNT]) {
        if self.channel_setting == 0 || data1 > 127 || data2 > 127 {
            return;
        }
        let channel = status & 0x0f;
        if self.channel_setting != 1 && self.channel_setting != channel + 2 {
            return;
        }
        self.release_disabled(enabled);
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
                    .groups
                    .iter()
                    .position(|group| group.held && group.note == data1 && group.channel == channel)
                {
                    self.release(index);
                }
            }
            0xb0 if data1 == 123 || data1 == 120 => {
                for index in 0..GROUP_COUNT {
                    if self
                        .groups
                        .get(index)
                        .is_some_and(|group| group.channel == channel)
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
    fn note_on(&mut self, channel: u8, note: u8, enabled: &[bool; GROUP_COUNT]) {
        // Retrigger an already-held key; otherwise prefer fully idle groups,
        // then released groups, then steal the oldest held note (order zero).
        let index = self
            .groups
            .iter()
            .position(|v| v.held && v.channel == channel && v.note == note)
            .or_else(|| {
                self.groups
                    .iter()
                    .zip(enabled)
                    .position(|(v, &on)| on && !v.held && v.gain == 0.0 && !v.switching)
            })
            .or_else(|| {
                self.groups
                    .iter()
                    .zip(enabled)
                    .position(|(v, &on)| on && !v.held)
            })
            .or_else(|| {
                self.groups
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
        let newest = self.groups.iter().filter(|v| v.held).count() as u8;
        if let Some(group) = self.groups.get_mut(index) {
            group.note = note;
            group.channel = channel;
            group.order = newest;
            group.held = true;
            group.switching = group.gain > 0.0;
            if !group.switching {
                group.playing_note = note;
                group.playing_channel = channel;
                group.retune = true;
            }
        }
    }
}

#[cfg(test)]
#[path = "midi_tests.rs"]
mod tests;
