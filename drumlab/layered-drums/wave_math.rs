use core::f32::consts::{LOG2_E, PI};
use distingnt_api::api::math::exp2;

pub(crate) fn wrap(turns: f32) -> f32 {
    turns - turns as u32 as f32
}

#[inline(never)]
pub(crate) fn sine(turns: f32) -> f32 {
    // Reduce to +/-pi/2 before evaluating the odd polynomial.
    let angle = (4.0 * (wrap(turns + 0.25) - 0.5).abs() - 1.0) * (-PI * 0.5);
    let squared = angle * angle;
    angle
        * (1.0
            + squared
                * (-1.0 / 6.0
                    + squared * (1.0 / 120.0 + squared * (-1.0 / 5040.0 + squared / 362880.0))))
}

pub(crate) fn positive_pow(value: f32, power: f32) -> f32 {
    if value == 0.0 {
        return 0.0;
    }
    // Decompose into a power of two and a mantissa in [1, 2).
    let bits = value.to_bits();
    let exponent = (bits >> 23) as i32 - 127;
    let mantissa = f32::from_bits((bits & 0x7f_ffff) | 0x3f80_0000);
    let x = (mantissa - 1.0) / (mantissa + 1.0);
    let squared = x * x;
    let log = 2.0
        * x
        * (1.0
            + squared
                * (1.0 / 3.0 + squared * (1.0 / 5.0 + squared * (1.0 / 7.0 + squared / 9.0))));
    exp2((exponent as f32 + LOG2_E * log) * power)
}

#[derive(Clone, Copy)]
pub(crate) struct DampedMode {
    phase_step: f32,
    log2_step: f32,
    level: f32,
    sin_step: f32,
    cos_step: f32,
    sin: f32,
    cos: f32,
}

impl DampedMode {
    pub(crate) fn new((phase_step, log2_step, level): (f32, f32, f32)) -> Self {
        let ratio = exp2(log2_step);
        let half_sine = sine(phase_step * 0.5);
        Self {
            phase_step,
            log2_step,
            level,
            sin_step: sine(phase_step) * ratio,
            cos_step: (1.0 - 2.0 * half_sine * half_sine) * ratio,
            sin: 0.0,
            cos: level,
        }
    }

    pub(crate) fn anchor(&mut self, frame: usize) {
        let phase = self.phase_step * frame as f32;
        let amplitude = self.level * exp2(self.log2_step * frame as f32);
        self.sin = sine(phase) * amplitude;
        self.cos = sine(phase + 0.25) * amplitude;
    }

    #[allow(dead_code)]
    pub(crate) fn anchor_with_hold(&mut self, frame: usize, hold: usize) {
        if hold == 0 {
            self.anchor(frame);
            return;
        }
        let half_sine = sine(self.phase_step * 0.5);
        let loss = if frame < hold {
            1.0
        } else {
            exp2(self.log2_step)
        };
        self.sin_step = sine(self.phase_step) * loss;
        self.cos_step = (1.0 - 2.0 * half_sine * half_sine) * loss;
        let phase = self.phase_step * frame as f32;
        let amplitude = self.level * exp2(self.log2_step * frame.saturating_sub(hold) as f32);
        self.sin = sine(phase) * amplitude;
        self.cos = sine(phase + 0.25) * amplitude;
    }

    pub(crate) fn next(&mut self) -> f32 {
        let sample = self.sin;
        self.sin = sample * self.cos_step + self.cos * self.sin_step;
        self.cos = self.cos * self.cos_step - sample * self.sin_step;
        sample
    }
}
