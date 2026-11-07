//! Temporary USB profiling stream, enabled only while a monitor requests it.
use core::sync::atomic::{AtomicBool, Ordering};
static STREAMING: AtomicBool = AtomicBool::new(false);
pub const PACKET_LEN: usize = 114;

// Non-commercial manufacturer ID 7D, Wavesmith identifier WS, protocol 1.
// Commands include F0/F7 as supplied by the firmware's factory-wide callback.
pub fn control(bytes: &[u8]) {
    match bytes {
        [0xf0, 0x7d, b'W', b'S', 1, 0x10, 0xf7] => STREAMING.store(true, Ordering::Relaxed),
        [0xf0, 0x7d, b'W', b'S', 1, 0x11, 0xf7] => STREAMING.store(false, Ordering::Relaxed),
        _ => {}
    }
}
pub fn encode(mut value: u64, bytes: &mut [u8]) {
    for byte in bytes {
        *byte = (value & 0x7f) as u8;
        value >>= 7;
    }
}
#[derive(Clone, Copy)]
pub struct Telemetry {
    frames: u32,
    cycles: u64,
    stages: [u64; 5],
    callbacks: u32,
    sequence: u32,
    // UI owns these wrapping totals; audio only snapshots them for reporting.
    // The monitor takes differences, including wrap, between consecutive reports.
    draw_cycles: u32,
    draw_calls: u32,
}
impl Telemetry {
    pub const fn new() -> Self {
        Self {
            frames: 0,
            cycles: 0,
            stages: [0; 5],
            callbacks: 0,
            sequence: 0,
            draw_cycles: 0,
            draw_calls: 0,
        }
    }
    pub fn record_draw(&mut self, cycles: u32) {
        self.draw_cycles = self.draw_cycles.wrapping_add(cycles);
        self.draw_calls = self.draw_calls.wrapping_add(1);
    }
    pub fn record(
        &mut self,
        cycles: u32,
        frames: usize,
        sample_rate: u32,
        average: u32,
        voices: u8,
        midi: bool,
        stereo: bool,
        stages: [u32; 5],
    ) -> Option<[u8; PACKET_LEN]> {
        self.record_enabled(
            STREAMING.load(Ordering::Relaxed),
            cycles,
            frames,
            sample_rate,
            average,
            voices,
            midi,
            stereo,
            stages,
        )
    }
    fn record_enabled(
        &mut self,
        enabled: bool,
        cycles: u32,
        frames: usize,
        sample_rate: u32,
        average: u32,
        voices: u8,
        midi: bool,
        stereo: bool,
        stages: [u32; 5],
    ) -> Option<[u8; PACKET_LEN]> {
        if !enabled || sample_rate == 0 {
            self.frames = 0;
            self.cycles = 0;
            self.stages = [0; 5];
            self.callbacks = 0;
            return None;
        }
        self.frames = self.frames.saturating_add(frames as u32);
        self.cycles += cycles as u64;
        for (total, cycles) in self.stages.iter_mut().zip(stages) {
            *total += cycles as u64;
        }
        self.callbacks += 1;
        if self.frames < sample_rate {
            return None;
        }
        let mut packet = [0u8; PACKET_LEN];
        packet[..6].copy_from_slice(&[0xf0, 0x7d, b'W', b'S', 1, 3]);
        // 6..11 is filled with the instance index by the callback after releasing
        // all borrowed state. All numeric fields are little-endian base 128.
        encode(self.sequence as u64, &mut packet[11..16]);
        encode(average as u64, &mut packet[16..21]);
        encode(self.cycles, &mut packet[21..31]);
        encode(self.callbacks as u64, &mut packet[31..36]);
        encode(self.frames as u64, &mut packet[36..41]);
        encode(sample_rate as u64, &mut packet[41..46]);
        encode(frames as u64, &mut packet[46..51]);
        packet[51] = voices.min(10);
        packet[52] = u8::from(midi);
        packet[53] = u8::from(stereo);
        for (bytes, total) in packet[54..104].chunks_exact_mut(10).zip(self.stages) {
            encode(total, bytes);
        }
        // Cumulative UI counters are never reset from the audio callback.
        encode(self.draw_cycles as u64, &mut packet[104..109]);
        encode(self.draw_calls as u64, &mut packet[109..114]);
        self.sequence = self.sequence.wrapping_add(1);
        // Whole callbacks are included; their actual frame count is transmitted.
        self.frames = 0;
        self.cycles = 0;
        self.stages = [0; 5];
        self.callbacks = 0;
        Some(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn telemetry_uses_audio_time_encodes_exact_counts_and_resets_when_stopped() {
        let mut meter = Telemetry::new();
        meter.record_draw(456);
        for _ in 0..1999 {
            assert!(meter
                .record_enabled(
                    true,
                    1000,
                    24,
                    48000,
                    999,
                    10,
                    false,
                    true,
                    [2000, 100, 200, 300, 400]
                )
                .is_none());
        }
        let packet = meter
            .record_enabled(
                true,
                1000,
                24,
                48000,
                999,
                10,
                false,
                true,
                [2000, 100, 200, 300, 400],
            )
            .unwrap();
        let decode = |bytes: &[u8]| {
            bytes
                .iter()
                .enumerate()
                .fold(0u64, |value, (i, &b)| value | ((b as u64) << (7 * i)))
        };
        assert_eq!(&packet[..6], &[0xf0, 0x7d, b'W', b'S', 1, 3]);
        assert!(packet[1..].iter().all(|&b| b < 128));
        assert_eq!(decode(&packet[16..21]), 999);
        assert_eq!(decode(&packet[21..31]), 2_000_000);
        assert_eq!(decode(&packet[31..36]), 2000);
        assert_eq!(decode(&packet[36..41]), 48000);
        assert_eq!(decode(&packet[54..64]), 4_000_000);
        assert_eq!(decode(&packet[94..104]), 800_000);
        assert_eq!(&packet[51..54], &[10, 0, 1]);
        assert_eq!(decode(&packet[104..109]), 456);
        assert_eq!(decode(&packet[109..114]), 1);
        meter.record_enabled(true, 123, 24, 48000, 0, 1, true, false, [0; 5]);
        assert!(meter
            .record_enabled(false, 123, 24, 48000, 0, 1, true, false, [0; 5])
            .is_none());
        assert_eq!((meter.frames, meter.cycles, meter.callbacks), (0, 0, 0));
    }
}
