//! Value-only instance state, validated channel count, and audio-driven UI clock.
#![forbid(unsafe_code)]
use super::ui::UiState;
use core::num::{NonZeroU32, NonZeroUsize};

#[derive(Clone, Copy)]
pub(super) struct State {
    pub num_channels: ChannelCount,
    pub first_step: bool,
    pub range_min: i8,
    pub range_max: i8,
    pub range: u8,
    pub clock: AudioClock,
    pub ui: UiState,
}

#[derive(Clone, Copy)]
pub(super) struct AudioClock {
    remaining_frames: u32,
    samples_per_ms: NonZeroU32,
    total_ms: u32,
}

impl AudioClock {
    pub fn new(sample_rate: u32) -> Option<Self> {
        Some(Self {
            remaining_frames: 0,
            // Preserve C++ integer truncation, including at 44.1 kHz.
            samples_per_ms: NonZeroU32::new(sample_rate / 1000)?,
            total_ms: 0,
        })
    }

    pub fn advance(&mut self, frames: u32) {
        self.remaining_frames = self.remaining_frames.wrapping_add(frames);
        let elapsed = self.remaining_frames / self.samples_per_ms;
        self.total_ms = self.total_ms.wrapping_add(elapsed);
        self.remaining_frames -= elapsed * self.samples_per_ms.get();
    }

    pub fn now(&self) -> u32 {
        self.total_ms
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;

    #[test]
    fn partial_milliseconds_survive_varying_block_sizes() {
        let mut clock = AudioClock::new(48_000).unwrap();
        let mut frames = 0;
        for block in [4, 16, 48, 128].into_iter().cycle().take(100) {
            frames += block;
            clock.advance(block);
            assert_eq!(clock.now(), frames / 48);
            assert_eq!(clock.remaining_frames, frames % 48);
        }
    }

    #[test]
    fn sample_rate_validation_and_truncation_match_timekeeper() {
        assert!(AudioClock::new(0).is_none());
        assert!(AudioClock::new(999).is_none());
        for (rate, divisor) in [(1000, 1), (44_100, 44), (48_000, 48), (96_000, 96)] {
            let mut clock = AudioClock::new(rate).unwrap();
            clock.advance(divisor - 1);
            assert_eq!(clock.now(), 0);
            clock.advance(1);
            assert_eq!(clock.now(), 1);
        }
    }

    #[test]
    fn counters_wrap_like_unsigned_cpp_counters() {
        let mut clock = AudioClock::new(48_000).unwrap();
        clock.total_ms = u32::MAX;
        clock.advance(49);
        assert_eq!(clock.now(), 0);
        assert_eq!(clock.remaining_frames, 1);

        // Even an unusually large block retains the original u32 addition.
        clock.advance(u32::MAX);
        assert_eq!(clock.now(), 0);
        assert_eq!(clock.remaining_frames, 0);
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ChannelCount(NonZeroUsize);

impl ChannelCount {
    pub const fn new(value: usize) -> Option<Self> {
        if value > 8 {
            return None;
        }
        match NonZeroUsize::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

#[cfg(test)]
mod channel_count_tests {
    use super::*;
    #[test]
    fn channel_counts_cannot_represent_invalid_specifications() {
        for count in 1..=8 {
            assert_eq!(ChannelCount::new(count).unwrap().get(), count);
        }
        for count in [0, 9, usize::MAX] {
            assert!(ChannelCount::new(count).is_none());
        }
    }
}
