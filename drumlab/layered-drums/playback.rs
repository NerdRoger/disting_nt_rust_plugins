use distingnt_api::{
    api::plugin::{RegionInitializer, Specification},
    TYPE_GENERIC,
};

use crate::WaveBuffers;

pub(crate) const fn output_buffers_specification(min: i32, max: i32, default: i32) -> Specification {
    assert!(min > 0 && min <= default && default <= max);
    // The player tracks one bit per output slot.
    assert!(max <= u16::BITS as i32);
    Specification {
        name: c"Output Buffers",
        min,
        max,
        default,
        kind: TYPE_GENERIC,
    }
}

pub(crate) fn output_buffer_count(specification: &Specification, count: i32) -> Option<usize> {
    (specification.min..=specification.max)
        .contains(&count)
        .then_some(count as usize)
}

const VOICES: usize = 8;
pub const OUTPUT_VOLTS: f32 = 5.0;
const BUFFER_WINDOW_SECONDS: u32 = 5;
const WAIT_BUCKETS_PER_SECOND: u32 = 20;
// Retain the current partial bucket so a wait is never dropped early.
const WAIT_BUCKETS: usize = (BUFFER_WINDOW_SECONDS * WAIT_BUCKETS_PER_SECOND + 1) as usize;

#[derive(Clone, Copy)]
pub struct BufferUsage<const OCCUPANCY_COUNTS: usize> {
    frame: u64,
    expires_at: [u64; OCCUPANCY_COUNTS],
    wait_buckets: [u16; WAIT_BUCKETS],
    wait_bucket: usize,
    wait_bucket_frames: u32,
    waits: i32,
    waiting_for_buffer: bool,
}

impl<const OCCUPANCY_COUNTS: usize> BufferUsage<OCCUPANCY_COUNTS> {
    #[cfg(test)]
    pub const EMPTY: Self = Self {
        frame: 0,
        expires_at: [0; OCCUPANCY_COUNTS],
        wait_buckets: [0; WAIT_BUCKETS],
        wait_bucket: 0,
        wait_bucket_frames: 0,
        waits: 0,
        waiting_for_buffer: false,
    };

    pub fn initialize(destination: &mut RegionInitializer<'_, Self>) {
        distingnt_api::initialize_region!(
            destination,
            Self {
                frame: 0,
                wait_bucket: 0,
                wait_bucket_frames: 0,
                waits: 0,
                waiting_for_buffer: false,
            },
            arrays {
                expires_at: |_| 0,
                wait_buckets: |_| 0
            }
        );
    }

    pub fn observe(&mut self, used: usize, frames: usize, rate: u32) {
        self.frame += frames as u64;
        // Keep the latest observation for each distinct count. A lower count
        // can take over immediately when a higher one ages out of the window.
        if let Some(expiry) = self.expires_at.get_mut(used) {
            *expiry = self.frame + u64::from(rate) * u64::from(BUFFER_WINDOW_SECONDS);
        }
        let bucket_frames = rate.div_ceil(WAIT_BUCKETS_PER_SECOND).max(1);
        self.wait_bucket_frames += frames as u32;
        while self.wait_bucket_frames >= bucket_frames {
            self.wait_bucket_frames -= bucket_frames;
            self.wait_bucket = (self.wait_bucket + 1) % WAIT_BUCKETS;
            if let Some(expired) = self.wait_buckets.get_mut(self.wait_bucket) {
                self.waits -= i32::from(core::mem::take(expired));
            }
        }
    }

    pub fn next_render_slot(&mut self, player: &Player, output_buffers: usize) -> Option<usize> {
        let slot = player.free_buffer(output_buffers);
        if slot.is_none() && !self.waiting_for_buffer {
            self.record_wait();
        }
        self.waiting_for_buffer = slot.is_none();
        slot
    }

    fn record_wait(&mut self) {
        if let Some(bucket) = self.wait_buckets.get_mut(self.wait_bucket) {
            *bucket += 1;
            self.waits += 1;
        }
    }

    pub fn waits(&self) -> i32 {
        self.waits
    }

    pub fn peak(&self) -> i32 {
        self.expires_at
            .iter()
            .rposition(|&expiry| expiry > self.frame)
            .unwrap_or(0) as i32
    }
}

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

    pub fn has_waveform(&self) -> bool {
        self.current.active()
    }

    fn free_buffer(&self, output_buffers: usize) -> Option<usize> {
        let slot = (!self.occupied_buffers()).trailing_zeros() as usize;
        (slot < output_buffers).then_some(slot)
    }

    // Sample before playback, which can release slots while mixing the block.
    pub fn used_buffers(&self, in_flight: Option<usize>) -> usize {
        let mut slots = self.occupied_buffers();
        if let Some(slot) = in_flight {
            slots |= 1 << slot;
        }
        slots.count_ones() as usize
    }

    fn occupied_buffers(&self) -> u16 {
        let mut slots = 0;
        for hit in self.voices.iter().chain([&self.current, &self.fading]) {
            if hit.active() {
                slots |= 1 << hit.slot;
            }
        }
        slots
    }

    fn trigger(&mut self) {
        if !self.has_waveform() {
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

    const TEST_POOL_SIZE: usize = 6;

    #[test]
    fn smaller_plugin_can_choose_its_own_buffer_limits_and_default() {
        const SPECIFICATION: Specification = output_buffers_specification(2, 5, 3);
        assert_eq!(
            output_buffer_count(&SPECIFICATION, SPECIFICATION.default),
            Some(3)
        );
        assert_eq!(output_buffer_count(&SPECIFICATION, 5), Some(5));
        assert_eq!(output_buffer_count(&SPECIFICATION, 1), None);
        assert_eq!(output_buffer_count(&SPECIFICATION, 6), None);

        let mut usage = BufferUsage::<{ SPECIFICATION.max as usize + 1 }>::EMPTY;
        usage.observe(5, 1, 100);
        assert_eq!(usage.peak(), 5);
    }

    #[test]
    fn buffer_usage_keeps_lower_peaks_after_a_higher_one_expires() {
        let mut usage = BufferUsage::<{ TEST_POOL_SIZE + 1 }>::EMPTY;
        assert_eq!(usage.peak(), 0);
        usage.observe(4, 12, 100);
        usage.observe(2, 499, 100);
        assert_eq!(usage.peak(), 4);
        usage.observe(2, 1, 100);
        assert_eq!(usage.peak(), 2);
        usage.observe(0, 500, 100);
        assert_eq!(usage.peak(), 0);
    }

    #[test]
    fn buffer_waits_expire_independently_after_five_seconds() {
        let mut usage = BufferUsage::<{ TEST_POOL_SIZE + 1 }>::EMPTY;
        usage.record_wait();
        usage.observe(2, 17, 100);
        usage.record_wait();
        usage.observe(2, 483, 100);
        assert_eq!(usage.waits(), 2);
        usage.observe(2, 5, 100);
        assert_eq!(usage.waits(), 1);
        usage.observe(2, 15, 100);
        assert_eq!(usage.waits(), 0);
        usage.record_wait();
        assert_eq!(usage.waits(), 1);
    }

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
        assert_eq!(player.free_buffer(TEST_POOL_SIZE), Some(1));
        player.publish(1, 200);
        player.trigger();
        player.trigger();
        assert_eq!(player.free_buffer(TEST_POOL_SIZE), Some(2));
        assert_eq!(player.used_buffers(None), 2);
        assert_eq!(player.used_buffers(Some(1)), 2);
        assert_eq!(player.used_buffers(Some(2)), 3);
        player.publish(2, 300);
        for next in 3..TEST_POOL_SIZE {
            player.trigger();
            assert_eq!(player.free_buffer(TEST_POOL_SIZE), Some(next));
            player.publish(next, 300);
        }
        assert_eq!(player.free_buffer(TEST_POOL_SIZE), None);
        assert_eq!(player.voices[0].slot, 0);
        assert_eq!(player.voices[1].slot, 1);
        assert_eq!(player.voices[2].slot, 1);
        player.voices[0].position = 100;
        assert_eq!(player.free_buffer(TEST_POOL_SIZE), Some(0));
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
        assert_eq!(run(TEST_POOL_SIZE), (0, 0));
    }
}
