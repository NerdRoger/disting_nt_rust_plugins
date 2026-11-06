//! Optional oscillators sharing their parent's note and gate, with independent pan.
use super::{dsp, stereo};

pub const SUBS: usize = 2;

#[derive(Clone, Copy)]
pub struct Settings {
    pub active: bool,
    pub octaves: f32,
    pub pan: i16,
}
impl Settings {
    pub const OFF: Self = Self {
        active: false,
        octaves: 0.0,
        pan: 50,
    };
}

#[derive(Clone, Copy)]
pub struct SubVoice {
    pub oscillator: dsp::Oscillator,
    pub pan: stereo::Pan,
    gain: f32,
}
impl SubVoice {
    pub const fn new() -> Self {
        Self {
            oscillator: dsp::Oscillator::new(),
            pan: stereo::Pan::new(),
            gain: 0.0,
        }
    }
    pub fn silence(&mut self) {
        self.gain = 0.0;
    }
    pub fn reset(&mut self) {
        self.oscillator = dsp::Oscillator::new();
        self.gain = 0.0;
    }
    pub fn rendering(&self, active: bool) -> bool {
        active || self.gain > 0.0
    }

    /// Render one independent oscillator using the parent's already-advanced gate.
    /// Return its squared gate weight for the shared loudness controller.
    pub fn render(
        &mut self,
        engine: &dsp::Engine,
        memory: &dsp::Memory,
        output: &mut [f32],
        gates: &[f32],
        parent_steady: bool,
        active: bool,
        gate_step: f32,
        frame_start: usize,
    ) -> f32 {
        output.fill(0.0);
        if active
            && self.gain == 1.0
            && parent_steady
            && self.oscillator.render_open_gate(engine, memory, output)
        {
            return output.len() as f32;
        }
        let mut power = 0.0;
        for (frame, (sample, &parent_gate)) in output.iter_mut().zip(gates).enumerate() {
            self.gain = if active {
                (self.gain + gate_step).min(1.0)
            } else {
                (self.gain - gate_step).max(0.0)
            };
            let gate = parent_gate * self.gain;
            if gate > 0.0 {
                power += gate * gate;
                *sample = gate
                    * self
                        .oscillator
                        .tick_prepared(engine, memory, frame_start + frame);
            }
        }
        power
    }
}
