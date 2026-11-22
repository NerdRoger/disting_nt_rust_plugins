//! Feature-gated NTP v1 development telemetry.
use crate::playback::{PlaybackDiagnostics, OUTPUT_VOLTS};
use core::sync::atomic::{AtomicU32, Ordering};
use distingnt_api::api::cpu_cycle_count;

#[derive(Clone, Copy)]
pub struct RenderTiming {
    pub total: Option<u32>,
    pub synthesis: [Option<u32>; crate::MAX_LAYERS],
    pub lowpass: Option<u32>,
    pub gain_and_peak: Option<u32>,
    pub mix: u32,
    pub scope: Option<u32>,
    pub grains: [GrainTiming; crate::MAX_LAYERS],
}
#[derive(Clone, Copy, Default)]
pub struct GrainTiming {
    pub accumulation: Option<u32>,
    pub shaping: Option<u32>,
}
pub const NO_RENDER_TIMING: RenderTiming = RenderTiming {
    total: None,
    synthesis: [None; crate::MAX_LAYERS],
    lowpass: None,
    gain_and_peak: None,
    mix: 0,
    scope: None,
    grains: [GrainTiming {
        accumulation: None,
        shaping: None,
    }; crate::MAX_LAYERS],
};

pub const STEP: usize = 0;
pub const SCHEDULE: usize = 1;
pub const RENDER: usize = 2;
pub const PARAMETER_CHANGED: usize = 3;
const SYNTHESIS: usize = 4;
const LOWPASS: usize = 5;
const GAIN_AND_PEAK: usize = 6;
pub const PUBLISH: usize = 7;
pub const SUBMIT: usize = 8;
const CALLBACK_TOTAL: usize = 9;
const OUTSIDE_CALLBACK: usize = 10;
pub const TELEMETRY_SEND: usize = 11;
const GRAIN_ACCUMULATION: usize = 12;
const GRAIN_SHAPING: usize = 13;
const MIX: usize = 14;
const SCOPE: usize = 15;
pub(crate) const FUNCTIONS: [&[u8]; 16] = [
    b"KickLab::step",
    b"KickLab::process_render",
    b"KickLab::render::render",
    b"KickLab::parameter_changed",
    b"KickLab::render::layer_synthesis",
    b"KickLab::render::master_lowpass",
    b"KickLab::render::gain_and_peak",
    b"KickLab::publish_waveform",
    b"KickLab::snapshot_and_submit",
    b"KickLab::callback_total",
    b"KickLab::outside_callback",
    b"KickLab::profiling_midi_send",
    b"KickLab::render::grain_accumulation",
    b"KickLab::render::grain_shaping",
    b"KickLab::render::mix_layers",
    b"KickLab::render::scope",
];
const FUNCTION_COUNT: usize = FUNCTIONS.len();
const METADATA_PACKETS: usize = 1 + FUNCTION_COUNT;
#[cfg(test)]
const TIMING_PACKET_BYTES: usize = 71;
const MAX_PACKET_BYTES: usize = 120;
static NEXT_SESSION: AtomicU32 = AtomicU32::new(1);

// Hash the sources used for this profiling build, including the worker facade.
const BUILD: u32 = build_id(&[
    include_bytes!("lib.rs"),
    include_bytes!("lookup.rs"),
    include_bytes!("profile.rs"),
    include_bytes!("playback.rs"),
    include_bytes!("render.rs"),
    include_bytes!("scope.rs"),
    include_bytes!("extra_models.rs"),
    include_bytes!("spectral.rs"),
    include_bytes!("../Cargo.toml"),
    include_bytes!("../../Cargo.lock"),
    include_bytes!("../../.cargo/config.toml"),
    include_bytes!("../../distingnt-api/src/api/plugin.rs"),
    include_bytes!("../../distingnt-api/src/api/background.rs"),
    include_bytes!("../../distingnt-api/src/api/static_dram.rs"),
]) ^ cfg!(feature = "profiling-hold-playback") as u32
    ^ ((cfg!(feature = "profiling-no-click-rebuild") as u32) << 1);

const fn build_id(sources: &[&[u8]]) -> u32 {
    let mut hash = 0x811c9dc5u32;
    let mut source = 0;
    while source < sources.len() {
        let mut index = 0;
        while index < sources[source].len() {
            hash = (hash ^ sources[source][index] as u32).wrapping_mul(0x01000193);
            index += 1;
        }
        source += 1;
    }
    hash
}

#[derive(Clone, Copy)]
struct Stats {
    calls: u32,
    total: u64,
    min: u32,
    max: u32,
    dropped: u32,
}

impl Stats {
    const EMPTY: Self = Self {
        calls: 0,
        total: 0,
        min: u32::MAX,
        max: 0,
        dropped: 0,
    };

    fn record(&mut self, cycles: u32) {
        if self.calls == u32::MAX {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.calls += 1;
        // A sum of at most u32::MAX u32 values fits in u64.
        self.total += u64::from(cycles);
        self.min = self.min.min(cycles);
        self.max = self.max.max(cycles);
    }
}

#[derive(Clone, Copy)]
struct Window {
    id: u32,
    start: u64,
    duration: u32,
    stats: [Stats; FUNCTION_COUNT],
    diagnostics: Diagnostics,
    next: usize,
}

#[derive(Clone, Copy)]
struct Diagnostics {
    gaps: Stats,
    missed_reads: u32,
    skipped_voice_frames: u32,
    worker_wait_frames: u32,
    buffer_wait_frames: u32,
    peak_mv: u32,
    above_one_hit_blocks: u32,
    add_mode_blocks: u32,
}

impl Diagnostics {
    const EMPTY: Self = Self {
        gaps: Stats::EMPTY,
        missed_reads: 0,
        skipped_voice_frames: 0,
        worker_wait_frames: 0,
        buffer_wait_frames: 0,
        peak_mv: 0,
        above_one_hit_blocks: 0,
        add_mode_blocks: 0,
    };
}

#[derive(Clone, Copy)]
pub struct Meter {
    identity: Option<(u32, u32)>,
    first_block: u32,
    sequence: u32,
    window: u32,
    frames: u32,
    fraction: u32,
    start: u64,
    stats: [Stats; FUNCTION_COUNT],
    previous_entry: Option<u32>,
    previous_callback: Option<(u32, u32)>,
    diagnostics: Diagnostics,
    pending: Option<Window>,
}

impl Meter {
    pub const EMPTY: Self = Self {
        identity: None,
        first_block: 0,
        sequence: 0,
        window: 0,
        frames: 0,
        fraction: 0,
        start: 0,
        stats: [Stats::EMPTY; FUNCTION_COUNT],
        previous_entry: None,
        previous_callback: None,
        diagnostics: Diagnostics::EMPTY,
        pending: None,
    };

    pub fn record(&mut self, function: usize, cycles: u32) {
        if let Some(stats) = self.stats.get_mut(function) {
            stats.record(cycles);
        }
    }

    pub fn record_render(&mut self, timing: RenderTiming) {
        if let Some(cycles) = timing.total {
            self.record(RENDER, cycles);
            self.record(MIX, timing.mix);
        }
        for cycles in timing.synthesis.into_iter().flatten() {
            self.record(SYNTHESIS, cycles);
        }
        if let Some(cycles) = timing.lowpass {
            self.record(LOWPASS, cycles);
        }
        if let Some(cycles) = timing.gain_and_peak {
            self.record(GAIN_AND_PEAK, cycles);
        }
        if let Some(cycles) = timing.scope {
            self.record(SCOPE, cycles);
        }
        for grain in timing.grains {
            if let Some(cycles) = grain.accumulation {
                self.record(GRAIN_ACCUMULATION, cycles);
            }
            if let Some(cycles) = grain.shaping {
                self.record(GRAIN_SHAPING, cycles);
            }
        }
    }

    pub fn record_callback_entry(&mut self, entry: u32) {
        if let Some((previous_entry, previous_exit)) = self.previous_callback.take() {
            self.record(CALLBACK_TOTAL, previous_exit.wrapping_sub(previous_entry));
            self.record(OUTSIDE_CALLBACK, entry.wrapping_sub(previous_exit));
        }
    }

    pub fn record_callback_exit(&mut self, entry: u32, exit: u32) {
        self.previous_callback = Some((entry, exit));
    }

    pub fn record_playback(
        &mut self,
        entry: u32,
        read: PlaybackDiagnostics,
        peak: Option<f32>,
        replace: bool,
    ) {
        if let Some(previous) = self.previous_entry.replace(entry) {
            self.diagnostics.gaps.record(entry.wrapping_sub(previous));
        }
        let diagnostics = &mut self.diagnostics;
        diagnostics.missed_reads = diagnostics.missed_reads.saturating_add(read.missed_reads);
        diagnostics.skipped_voice_frames = diagnostics
            .skipped_voice_frames
            .saturating_add(read.skipped_voice_frames);
        if let Some(peak) = peak {
            diagnostics.peak_mv = diagnostics.peak_mv.max((peak * 1000.0 + 0.5) as u32);
            diagnostics.above_one_hit_blocks += u32::from(peak > OUTPUT_VOLTS);
            diagnostics.add_mode_blocks += u32::from(!replace);
        }
    }

    pub fn wait_for_worker(&mut self, frames: usize) {
        self.diagnostics.worker_wait_frames += frames as u32;
    }

    pub fn wait_for_buffer(&mut self, frames: usize) {
        self.diagnostics.buffer_wait_frames += frames as u32;
    }

    pub fn advance(&mut self, frames: usize, rate: u32) {
        let frames = frames as u32;
        if self.identity.is_none() {
            let instance = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
            self.identity = Some((
                instance & 0x3fff,
                cpu_cycle_count().rotate_left(7) ^ instance,
            ));
            self.first_block = frames;
        }
        if self.first_block != 0 && self.first_block != frames {
            // Clock coverage stays sample-exact. Begin a new, variable-size
            // stream if the host changes block length without reconstructing.
            let instance = self.identity.map_or(0, |identity| identity.0);
            let next = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
            let stats = self.stats.map(|entry| Stats {
                dropped: entry.dropped.saturating_add(entry.calls),
                ..Stats::EMPTY
            });
            *self = Self {
                identity: Some((instance, cpu_cycle_count().rotate_left(7) ^ next)),
                stats,
                ..Self::EMPTY
            };
        }
        self.frames = self.frames.saturating_add(frames);
        if self.frames < rate || rate == 0 {
            return;
        }

        // Convert frame time to microseconds while carrying sub-microsecond
        // remainders between intervals. Split the ratio to use 32-bit division.
        let ms = (self.frames % rate) * 1000;
        let us = (ms % rate) * 1000 + self.fraction;
        let duration = (self.frames / rate) * 1_000_000 + (ms / rate) * 1000 + us / rate;
        self.fraction = us % rate;
        self.frames = 0;
        if self.pending.is_none() {
            self.pending = Some(Window {
                id: self.window,
                start: self.start,
                duration,
                stats: self.stats,
                diagnostics: self.diagnostics,
                next: if self.window % 4 == 0 {
                    0
                } else {
                    METADATA_PACKETS
                },
            });
            self.stats = [Stats::EMPTY; FUNCTION_COUNT];
        } else {
            // Keep packets for the older interval intact; report calls lost
            // from this unsent interval with the next retained interval.
            for stats in &mut self.stats {
                *stats = Stats {
                    dropped: stats.dropped.saturating_add(stats.calls),
                    ..Stats::EMPTY
                };
            }
        }
        self.diagnostics = Diagnostics::EMPTY;
        self.start += u64::from(duration);
        self.window = self.window.wrapping_add(1);
    }

    pub fn packet(&mut self, rate: u32) -> Option<Packet> {
        let (instance, session) = self.identity?;
        let window = self.pending.as_mut()?;
        let mut packet = Packet {
            bytes: [0; MAX_PACKET_BYTES],
            len: 0,
        };
        let next = window.next;
        if next == METADATA_PACKETS + FUNCTION_COUNT {
            let diag = window.diagnostics;
            let render = window.stats.get(RENDER)?;
            // KLD v1 uses the same integer encoding and audio-clock window IDs
            // as NTP. Its separate header keeps volts and counts out of NTP.
            packet.append(&[0xf0, 0x7d, b'K', b'L', b'D', 1]);
            packet.number(BUILD.into(), 5);
            packet.number(instance.into(), 2);
            packet.number(session.into(), 5);
            packet.number(window.id.into(), 5);
            packet.number(window.start, 10);
            packet.number(window.duration.into(), 5);
            packet.number(diag.gaps.calls.into(), 5);
            packet.number(diag.gaps.total, 10);
            packet.number(
                if diag.gaps.calls == 0 {
                    0
                } else {
                    diag.gaps.min.into()
                },
                5,
            );
            packet.number(diag.gaps.max.into(), 5);
            packet.number(diag.missed_reads.into(), 5);
            packet.number(diag.skipped_voice_frames.into(), 5);
            packet.number(diag.worker_wait_frames.into(), 5);
            packet.number(diag.buffer_wait_frames.into(), 5);
            packet.number(diag.peak_mv.into(), 5);
            packet.number(diag.above_one_hit_blocks.into(), 5);
            packet.number(diag.add_mode_blocks.into(), 5);
            packet.number(rate.into(), 5);
            packet.number(self.first_block.min(0x3fff).into(), 2);
            packet.number(render.calls.into(), 5);
            packet.number(render.total, 10);
            self.pending = None;
            return Some(packet);
        }
        let kind = if next == 0 {
            0
        } else if next < METADATA_PACKETS {
            1
        } else {
            2
        };
        packet.append(&[0xf0, 0x7d, b'N', b'T', b'P', 1, kind]);
        packet.number(instance.into(), 2);
        packet.number(session.into(), 5);
        packet.number(self.sequence.into(), 5);
        match kind {
            0 => {
                packet.number(BUILD.into(), 5);
                packet.number(rate.into(), 5);
                packet.number(self.first_block.min(0x3fff).into(), 2);
                packet.number(0, 5); // Cycle-clock frequency has not been measured.
                packet.number(FUNCTION_COUNT as u64, 2);
            }
            1 => {
                packet.number((next - 1) as u64, 2);
                packet.append(FUNCTIONS.get(next - 1)?);
            }
            _ => {
                let index = next - METADATA_PACKETS;
                let stats = window.stats.get(index)?;
                packet.number(window.id.into(), 5);
                packet.number(index as u64, 2);
                packet.number(window.start, 10);
                packet.number(window.duration.into(), 5);
                packet.number(stats.calls.into(), 5);
                packet.number(stats.total, 10);
                packet.number(
                    if stats.calls == 0 {
                        0
                    } else {
                        stats.min.into()
                    },
                    5,
                );
                packet.number(stats.max.into(), 5);
                packet.number(stats.dropped.into(), 5);
            }
        }
        // One packet per callback. The host appends F7.
        self.sequence = self.sequence.wrapping_add(1);
        window.next += 1;
        Some(packet)
    }
}

pub struct Packet {
    bytes: [u8; MAX_PACKET_BYTES],
    len: usize,
}

impl Packet {
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }

    fn append(&mut self, data: &[u8]) {
        let end = self.len + data.len();
        if let Some(slot) = self.bytes.get_mut(self.len..end) {
            for (destination, &byte) in slot.iter_mut().zip(data) {
                *destination = byte;
            }
            self.len = end;
        }
    }

    fn number(&mut self, mut number: u64, width: usize) {
        let end = self.len + width;
        if let Some(slot) = self.bytes.get_mut(self.len..end) {
            for digit in slot {
                *digit = (number & 127) as u8;
                number >>= 7;
            }
            self.len = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(bytes: &[u8]) -> u64 {
        bytes
            .iter()
            .rev()
            .fold(0, |total, &digit| (total << 7) | u64::from(digit))
    }

    #[test]
    fn callback_timing_splits_execution_from_the_gap_across_counter_wrap() {
        let mut meter = Meter::EMPTY;
        meter.record_callback_entry(u32::MAX - 10);
        meter.record_callback_exit(u32::MAX - 10, 4);
        meter.record_callback_entry(24);
        assert_eq!(meter.stats[CALLBACK_TOTAL].total, 15);
        assert_eq!(meter.stats[OUTSIDE_CALLBACK].total, 20);
    }

    #[test]
    fn windows_encode_extrema_zeros_sequence_and_device_time() {
        let mut meter = Meter::EMPTY;
        meter.record_playback(
            u32::MAX - 10,
            PlaybackDiagnostics {
                missed_reads: 1,
                skipped_voice_frames: 3,
            },
            Some(3.2),
            false,
        );
        meter.record_playback(10, PlaybackDiagnostics::default(), Some(6.75), true);
        meter.wait_for_worker(24);
        meter.wait_for_buffer(48);
        for _ in 0..2000 {
            meter.record(STEP, 100);
            meter.advance(24, 48_000);
        }
        for seq in 0..(METADATA_PACKETS + FUNCTION_COUNT) {
            let kind = if seq == 0 {
                0
            } else if seq < METADATA_PACKETS {
                1
            } else {
                2
            };
            let message = meter.packet(48_000).unwrap();
            let bytes = message.as_bytes();
            assert_eq!(&bytes[..6], &[0xf0, 0x7d, b'N', b'T', b'P', 1]);
            assert!(bytes[1..].iter().all(|&byte| byte < 128));
            assert_eq!(bytes[6], kind);
            assert_eq!(number(&bytes[14..19]), seq as u64);
            if kind == 2 {
                assert_eq!(bytes.len(), TIMING_PACKET_BYTES);
                assert_eq!(number(&bytes[26..36]), 0);
                assert_eq!(number(&bytes[36..41]), 1_000_000);
                let expected = if seq == METADATA_PACKETS { 2000 } else { 0 };
                assert_eq!(number(&bytes[41..46]), expected);
                assert_eq!(number(&bytes[46..56]), expected * 100);
                assert_eq!(number(&bytes[56..61]), if expected != 0 { 100 } else { 0 });
            }
        }
        let diagnostic = meter.packet(48_000).unwrap();
        let bytes = diagnostic.as_bytes();
        assert_eq!(bytes.len(), MAX_PACKET_BYTES);
        assert_eq!(&bytes[..6], &[0xf0, 0x7d, b'K', b'L', b'D', 1]);
        assert_eq!(number(&bytes[38..43]), 1);
        assert_eq!(number(&bytes[43..53]), 21);
        assert_eq!(number(&bytes[63..68]), 1);
        assert_eq!(number(&bytes[68..73]), 3);
        assert_eq!(number(&bytes[73..78]), 24);
        assert_eq!(number(&bytes[78..83]), 48);
        assert_eq!(number(&bytes[83..88]), 6750);
        assert_eq!(number(&bytes[88..93]), 1);
        assert_eq!(number(&bytes[93..98]), 1);
        assert!(meter.packet(48_000).is_none());
        for _ in 0..2000 {
            meter.advance(24, 48_000);
        }
        let message = meter.packet(48_000).unwrap();
        let bytes = message.as_bytes();
        assert_eq!(number(&bytes[26..36]), 1_000_000);
        assert_eq!(number(&bytes[41..46]), 0);
    }
}
