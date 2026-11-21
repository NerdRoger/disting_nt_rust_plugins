use distingnt_api::api::plugin::RegionInitializer;

use crate::WaveBuffers;

const VOICES: usize = 8;
pub const OUTPUT_VOLTS: f32 = 5.0;

#[derive(Default)]
pub struct PlaybackDiagnostics {
    #[cfg(feature = "profiling")]
    pub missed_reads: u32,
    #[cfg(feature = "profiling")]
    pub skipped_voice_frames: u32,
}

impl PlaybackDiagnostics {
    fn observe_read(&mut self, _read: Option<()>, _frames: usize) {
        #[cfg(feature = "profiling")]
        if _read.is_none() {
            self.missed_reads = self.missed_reads.saturating_add(1);
            self.skipped_voice_frames = self.skipped_voice_frames.saturating_add(_frames as u32);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hit {
    slot: usize,
    position: usize,
    length: usize,
}

impl Hit {
    const EMPTY: Self = Self {
        slot: 0,
        position: 0,
        length: 0,
    };

    // Passing the fields in registers lets the compiler emit direct stores.
    #[inline(never)]
    fn set(&mut self, slot: usize, position: usize, length: usize) {
        self.slot = slot;
        self.position = position;
        self.length = length;
    }

    fn active(self) -> bool {
        self.position < self.length
    }
}

#[derive(Clone, Copy)]
pub struct Player {
    current: Hit,
    voices: [Hit; VOICES],
    fading: Hit,
    fade_remaining: usize,
    fade_frames: usize,
    high: bool,
}

impl Player {
    pub fn initialize(destination: &mut RegionInitializer<'_, Self>, sample_rate: u32) {
        distingnt_api::initialize_region!(
            destination,
            Player {
                current: Hit::EMPTY,
                fading: Hit::EMPTY,
                fade_remaining: 0,
                fade_frames: (sample_rate as usize / 500).max(1),
                high: false,
            },
            arrays {
                voices: |_| Hit::EMPTY
            }
        );
    }

    pub fn publish(&mut self, slot: usize, length: usize) {
        self.current = Hit {
            slot,
            position: 0,
            length,
        };
    }

    #[cfg(feature = "profiling-hold-playback")]
    pub fn has_waveform(&self) -> bool {
        self.current.active()
    }

    pub fn free_buffer(&self, output_buffers: usize) -> Option<usize> {
        (0..output_buffers).find(|&slot| {
            !(self.current.active() && self.current.slot == slot
                || self.fading.active() && self.fading.slot == slot
                || self
                    .voices
                    .iter()
                    .any(|voice| voice.active() && voice.slot == slot))
        })
    }

    fn trigger(&mut self) {
        if !self.current.active() {
            return;
        }
        if let Some(voice) = self.voices.iter_mut().find(|voice| !voice.active()) {
            voice.set(
                self.current.slot,
                self.current.position,
                self.current.length,
            );
        } else if !self.fading.active() {
            let Some(oldest) = self.voices.iter_mut().max_by_key(|voice| voice.position) else {
                return;
            };
            self.fading.set(oldest.slot, oldest.position, oldest.length);
            self.fade_remaining = self.fade_frames;
            oldest.set(
                self.current.slot,
                self.current.position,
                self.current.length,
            );
        }
    }

    pub fn play(
        &mut self,
        buffers: &WaveBuffers,
        buses: &mut [f32],
        frames: usize,
        trigger: Option<usize>,
        output: Option<usize>,
        replace: bool,
        diagnostics: &mut PlaybackDiagnostics,
    ) {
        let trigger = trigger.map(|index| index * frames);
        let output = output.map(|index| index * frames);
        let mut start = 0;
        for frame in 0..frames {
            // Scan each input frame before writing the corresponding output,
            // including when the input and output use the same bus.
            let voltage = trigger
                .and_then(|index| buses.get(index + frame))
                .copied()
                .unwrap_or(0.0);
            if voltage >= 1.0 && !self.high {
                self.mix(buffers, buses, output, start..frame, replace, diagnostics);
                start = frame;
                self.trigger();
                self.high = true;
            } else if voltage <= 0.1 {
                self.high = false;
            }
        }
        self.mix(buffers, buses, output, start..frames, replace, diagnostics);
    }

    fn mix(
        &mut self,
        buffers: &WaveBuffers,
        buses: &mut [f32],
        output: Option<usize>,
        frames: core::ops::Range<usize>,
        replace: bool,
        diagnostics: &mut PlaybackDiagnostics,
    ) {
        if frames.is_empty() {
            return;
        }
        let length = frames.len();
        if let Some(samples) =
            output.and_then(|bus| buses.get_mut(bus + frames.start..bus + frames.end))
        {
            if replace {
                samples.fill(0.0);
            }
            for voice in &mut self.voices {
                let count = (voice.length - voice.position).min(length);
                if count != 0 {
                    let end = voice.position + count;
                    let read = buffers
                        .with(voice.slot, |wave| {
                            let wave = wave.get(voice.position..end)?;
                            for (out, &sample) in samples.iter_mut().zip(wave) {
                                *out += sample * OUTPUT_VOLTS;
                            }
                            Some(())
                        })
                        .flatten();
                    diagnostics.observe_read(read, count);
                    voice.position = end;
                }
            }
            if self.fading.active() {
                let hit = &mut self.fading;
                let count = (hit.length - hit.position)
                    .min(length)
                    .min(self.fade_remaining);
                let end = hit.position + count;
                let fade_remaining = self.fade_remaining;
                let scale = OUTPUT_VOLTS / self.fade_frames as f32;
                let read = buffers
                    .with(hit.slot, |wave| {
                        let wave = wave.get(hit.position..end)?;
                        for (index, (out, &sample)) in samples.iter_mut().zip(wave).enumerate() {
                            *out += sample * (fade_remaining - index) as f32 * scale;
                        }
                        Some(())
                    })
                    .flatten();
                diagnostics.observe_read(read, count);
            }
        } else {
            for voice in &mut self.voices {
                voice.position = (voice.position + length).min(voice.length);
            }
        }

        self.fading.position = (self.fading.position + length).min(self.fading.length);
        self.fade_remaining = self.fade_remaining.saturating_sub(length);
        if self.fade_remaining == 0 {
            self.fading = Hit::EMPTY;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_OUTPUT_BUFFERS;

    #[test]
    fn current_and_generations_of_tails_can_pin_every_buffer() {
        let mut player = Player {
            current: Hit::EMPTY,
            voices: [Hit::EMPTY; VOICES],
            fading: Hit::EMPTY,
            fade_remaining: 0,
            fade_frames: 96,
            high: false,
        };
        player.publish(0, 100);
        player.trigger();
        assert_eq!(player.free_buffer(DEFAULT_OUTPUT_BUFFERS), Some(1));
        player.publish(1, 200);
        player.trigger();
        player.trigger();
        assert_eq!(player.free_buffer(DEFAULT_OUTPUT_BUFFERS), Some(2));
        player.publish(2, 300);
        for next in 3..DEFAULT_OUTPUT_BUFFERS {
            player.trigger();
            assert_eq!(player.free_buffer(DEFAULT_OUTPUT_BUFFERS), Some(next));
            player.publish(next, 300);
        }
        assert_eq!(player.free_buffer(DEFAULT_OUTPUT_BUFFERS), None);
        assert_eq!(player.voices[0].slot, 0);
        assert_eq!(player.voices[1].slot, 1);
        assert_eq!(player.voices[2].slot, 1);
        player.voices[0].position = 100;
        assert_eq!(player.free_buffer(DEFAULT_OUTPUT_BUFFERS), Some(0));
    }

    #[test]
    fn long_overlapping_tails_can_keep_up_with_four_edits_per_second() {
        // The test advances the real buffer ownership in 10 ms ticks.
        // Model a 2.13 s waveform, 30 ms worker time, and 120 BPM triggers.
        let run = |capacity| {
            let mut player = Player {
                current: Hit::EMPTY,
                voices: [Hit::EMPTY; VOICES],
                fading: Hit::EMPTY,
                fade_remaining: 0,
                fade_frames: 1,
                high: false,
            };
            player.publish(0, 213);
            let (mut edited, mut published) = (0, 0);
            let mut pending = None;
            let (mut blocked, mut stale_hits) = (0, 0);
            for tick in 0..1200 {
                for voice in &mut player.voices {
                    voice.position = (voice.position + 1).min(voice.length);
                }
                if let Some((slot, generation, done)) = pending {
                    if tick == done {
                        player.publish(slot, 213);
                        published = generation;
                        pending = None;
                    }
                }
                if tick % 25 == 15 {
                    edited += 1;
                }
                if edited != published && pending.is_none() {
                    match player.free_buffer(capacity) {
                        Some(slot) => pending = Some((slot, edited, tick + 3)),
                        None => blocked += 1,
                    }
                }
                if tick % 50 == 0 {
                    player.trigger();
                    stale_hits += usize::from(edited != published);
                }
            }
            (blocked, stale_hits)
        };

        assert_eq!(run(3), (1080, 22));
        assert_eq!(run(DEFAULT_OUTPUT_BUFFERS), (0, 0));
    }
}
