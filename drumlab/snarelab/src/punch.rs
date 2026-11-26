use core::f32::consts::{LOG2_E, TAU};

use distingnt_api::api::math::exp2;

use super::{
    mix::db_gain,
    voice::{advance, decay_step, noise, Band},
    wave_math::{positive_pow, sine, wrap},
};
use crate::{lookup, CONTROLS};

fn smooth(value: f32) -> f32 {
    let x = value.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn exp(value: f32) -> f32 {
    exp2(LOG2_E * value)
}

fn unit_step(ms: f32, rate: u32) -> f32 {
    exp(-1000.0 / (ms * rate as f32))
}

fn blend(a: f32, b: f32, amount: f32) -> f32 {
    a * (1.0 - amount) + b * amount
}

pub(super) fn shape_punch(sample: f32, ms: f32) -> f32 {
    let dry = sample * (1.0 + 0.0961148 * (1.0 - smooth((ms - 4.0) / 10.0)));
    let squared = dry * dry;
    dry * positive_pow(1.0 + 0.24969612 * squared * squared, -0.25)
}

pub(super) fn body_duration(p: &[f32; CONTROLS]) -> f32 {
    p[3] + 0.01900435 + 2.7 * p[4]
}

pub(super) fn wires_duration(p: &[f32; CONTROLS]) -> f32 {
    p[0] + 7.814 * p[2]
}

pub(super) fn rim_duration(p: &[f32; CONTROLS]) -> f32 {
    blend(p[1] * 1.5 + 5.0, 10.0 * p[1], p[5] * 0.01).max(p[3] * 1.5 + 5.0)
}

#[derive(Clone, Copy)]
struct BandPass {
    feed: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl BandPass {
    fn new(hz: f32, q: f32, rate: u32) -> Self {
        let phase = hz / rate as f32;
        let alpha = sine(phase) / (2.0 * q);
        let normalization = (1.0 + alpha).recip();
        Self {
            feed: alpha * normalization,
            a1: 2.0 * sine(phase + 0.25) * normalization,
            a2: (alpha - 1.0) * normalization,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn sample(&mut self, input: f32) -> f32 {
        let feed = input * self.feed;
        let output = feed + self.z1;
        self.z1 = self.a1 * output + self.z2;
        self.z2 = self.a2 * output - feed;
        output
    }
}

#[derive(Clone, Copy)]
pub(super) struct TwoStageBody {
    hold: f32,
    knee: f32,
    gentle: f32,
    steep: f32,
    handoff: f32,
    head_step: f32,
    strike_step: f32,
    head_phase: f32,
    strike_phase: f32,
    strike_bend: f32,
    strike_decay: f32,
    bend: f32,
    bend_decay: f32,
    normalization: f32,
}

impl TwoStageBody {
    pub(super) fn new(p: &[f32; CONTROLS], rate: u32, tables: Option<lookup::Lookup<'_>>) -> Self {
        let length = p[4] * 0.01;
        let hold = p[3] + 0.01900435;
        let inverse_rate = (rate as f32).recip();
        Self {
            hold,
            knee: hold + 29.298264 * length,
            gentle: -0.14963674 / length,
            steep: -0.027775207 / (length * length),
            handoff: (p[2] - 0.02443202).recip(),
            head_step: (p[0] - 0.1379216) * inverse_rate,
            strike_step: (p[1] - 0.28217665) * inverse_rate,
            head_phase: 1.5 / TAU,
            strike_phase: 1.5 / TAU,
            strike_bend: 8.163301 * inverse_rate,
            strike_decay: unit_step(4.0, rate),
            bend: 67.0 * inverse_rate,
            bend_decay: unit_step(14.61175, rate),
            normalization: lookup::drive(tables, 1.4).recip(),
        }
    }

    pub(super) fn sample(&mut self, ms: f32, tables: Option<lookup::Lookup<'_>>) -> f32 {
        let bend = self.bend;
        if ms >= 14.400661 {
            self.bend *= self.bend_decay;
        }
        self.strike_phase = wrap(
            self.strike_phase
                + self.strike_step
                + advance(&mut self.strike_bend, self.strike_decay),
        );
        self.head_phase = wrap(self.head_phase + self.head_step + bend);
        let handoff = smooth((ms - 6.515377) * self.handoff);
        let body = sine(self.head_phase);
        let tone = if handoff < 1.0 {
            blend(sine(self.strike_phase), body, handoff)
        } else {
            body
        };
        let rise = smooth((ms - 4.2034683) / 5.3585324);
        let after_knee = (ms - self.knee).max(0.0);
        let fall =
            db_gain(self.gentle * (ms - self.hold).max(0.0) + self.steep * after_knee * after_knee);
        lookup::drive(tables, 1.4 * tone) * self.normalization * rise * fall
    }
}

#[derive(Clone, Copy)]
pub(super) struct ResonantWires {
    seed: u32,
    filters: [BandPass; 4],
    band_gain: [f32; 4],
    band_decay: [f32; 4],
    delay: f32,
    rise_age: f32,
    rise_decay: f32,
    ring: f32,
    ring_decay: f32,
    ring_pitch: f32,
    duration: f32,
}

impl ResonantWires {
    pub(super) fn new(p: &[f32; CONTROLS], rate: u32) -> Self {
        let length = p[2] * 0.01;
        let brightness = p[3] * 0.01;
        let delay = p[0] + 0.01610257;
        let delay_frames = delay * rate as f32 / 1000.0;
        let first_age = (1.0 + delay_frames as u32 as f32 - delay_frames) * 1000.0 / rate as f32;
        let dark = 144.92023 * length;
        let mid = 127.52979 * length;
        let high = 93.90853 * length;
        let air = 35.0 * length;
        let rise = p[1] - 0.0232118;
        let ring = 172.79385 * length;
        Self {
            seed: 0x1b546ba7 ^ (p[6] as u32).wrapping_mul(0x56d62e11),
            filters: [
                BandPass::new(1700.0, 2.0, rate),
                BandPass::new(2530.0, 1.4, rate),
                BandPass::new(6300.0, 0.85, rate),
                BandPass::new(10100.0, 1.4, rate),
            ],
            band_gain: [
                0.133 * exp(-first_age / dark),
                1.16 * exp(-first_age / mid),
                0.8637697 * brightness * exp(-first_age / high),
                0.056 * brightness * exp(-first_age / air),
            ],
            band_decay: [
                unit_step(dark, rate),
                unit_step(mid, rate),
                unit_step(high, rate),
                unit_step(air, rate),
            ],
            delay,
            rise_age: exp(-first_age / rise),
            rise_decay: unit_step(rise, rate),
            ring: 0.0002625 * p[5] * exp(-first_age / ring),
            ring_decay: unit_step(ring, rate),
            ring_pitch: (p[4] + 0.03966073) / 1000.0,
            duration: wires_duration(p),
        }
    }

    pub(super) fn sample(&mut self, ms: f32) -> f32 {
        let first = noise(&mut self.seed);
        let second = noise(&mut self.seed);
        let mut wires = 0.0;
        for ((filter, gain), input) in self
            .filters
            .iter_mut()
            .zip(&self.band_gain)
            .zip([first, second, first, second])
        {
            wires += filter.sample(input) * gain;
        }
        let age = ms - self.delay;
        if age <= 0.0 {
            return 0.0;
        }
        let rise = positive_pow(
            (1.0 - advance(&mut self.rise_age, self.rise_decay)).max(0.0),
            1.2152053,
        );
        let ring = advance(&mut self.ring, self.ring_decay)
            * smooth(age / 45.0)
            * sine(self.ring_pitch * age + 0.48 / TAU);
        for (gain, decay) in self.band_gain.iter_mut().zip(self.band_decay) {
            *gain *= decay;
        }
        (rise * wires + ring) * (1.0 - smooth((ms - (self.duration - 54.0)) / 54.0))
    }
}

#[derive(Clone, Copy)]
pub(super) struct Rim {
    length_inverse: f32,
    contour: f32,
    ms_step: f32,
    low_step: f32,
    upper_step: f32,
    low_phase: f32,
    upper_phase: f32,
    bend_step: f32,
    low_gain: f32,
    upper_gain: f32,
    snap_gain: f32,
    snap_step: f32,
    low_amp: f32,
    low_decay: f32,
    upper_amp: f32,
    upper_decay: f32,
    broad: Band,
    broad_gain: f32,
    broad_decay: f32,
    focused: BandPass,
    focused_gain: f32,
    focused_decay: f32,
    focused_seed: u32,
}

impl Rim {
    pub(super) fn new(p: &[f32; CONTROLS], rate: u32) -> Self {
        let contour = p[5] * 0.01;
        let focus = p[8] * 0.01;
        let pitch_scale = p[0] / 979.0;
        let inverse_rate = (rate as f32).recip();
        let low_step = blend(p[0], 978.8694 * pitch_scale, contour) * inverse_rate;
        let upper_step = blend(p[0] * 1.67, 1392.7303 * pitch_scale, contour) * inverse_rate;
        Self {
            length_inverse: 16.0 / p[1],
            contour,
            ms_step: 1000.0 * inverse_rate,
            low_step,
            upper_step,
            low_phase: contour * (0.25214174 / TAU + low_step),
            upper_phase: contour * (0.9880595 / TAU + upper_step),
            bend_step: p[6] * 0.01 * pitch_scale * inverse_rate,
            low_gain: blend(0.28, 0.3942595, contour),
            upper_gain: blend(0.16, 0.32295638, contour),
            snap_gain: 0.0045721 * p[7],
            snap_step: 1.5426723 * pitch_scale,
            low_amp: 1.0,
            low_decay: decay_step(p[1], rate),
            upper_amp: 1.0,
            upper_decay: decay_step(p[1] * 0.55, rate),
            broad: Band::new(1200.0, p[2], rate),
            broad_gain: 0.0063 * positive_pow(1.0 - focus, 0.5) * p[9],
            broad_decay: decay_step(p[3], rate),
            focused: BandPass::new(p[2], 1.5, rate),
            focused_gain: 0.0064 * positive_pow(focus, 0.5) * p[9],
            focused_decay: unit_step(8.0 * (p[3] / 36.8), rate),
            focused_seed: 0x57d4a4d8,
        }
    }

    pub(super) fn sample(&mut self, time_ms: f32, seed: &mut u32) -> f32 {
        let ms = time_ms * self.length_inverse;
        let low_now = self.low_phase;
        let upper_now = self.upper_phase;
        let next_ms = ms + self.ms_step * self.length_inverse;
        self.low_phase =
            wrap(low_now + self.low_step - 37.105915 * self.bend_step * smooth(next_ms / 13.0));
        self.upper_phase =
            wrap(upper_now + self.upper_step - 110.0 * self.bend_step * smooth(next_ms / 12.0));

        let mut low_env = advance(&mut self.low_amp, self.low_decay);
        let mut upper_env = advance(&mut self.upper_amp, self.upper_decay);
        if self.contour > 0.0 {
            let crack = ms / 7.1300464;
            let shaped_low = smooth(ms / 2.4044926)
                * db_gain(-10.0 * crack * crack - 0.270241 * ms)
                * exp(-(ms - 7.1300464).max(0.0) / 5.926603);
            let shaped_upper = smooth(ms / 1.5) * exp(-ms / 7.436961);
            low_env = blend(low_env, shaped_low, self.contour);
            upper_env = blend(upper_env, shaped_upper, self.contour);
        }
        let mut click = 0.0;
        if self.broad_gain > 0.000001 {
            click +=
                self.broad.sample(noise(seed)) * advance(&mut self.broad_gain, self.broad_decay);
        }
        if self.focused_gain > 0.000001 {
            click += self.focused.sample(noise(&mut self.focused_seed))
                * advance(&mut self.focused_gain, self.focused_decay)
                * smooth(time_ms / 0.4);
        }
        let snap_width = ms / 5.3;
        let snap = if self.snap_gain > 0.0 && snap_width < 5.0 {
            self.snap_gain
                * smooth(ms / 0.85)
                * exp(-snap_width * snap_width)
                * sine(self.snap_step * time_ms + 0.919646 / TAU)
        } else {
            0.0
        };
        blend((time_ms / 0.7).min(1.0), 1.0, self.contour)
            * (self.low_gain * sine(low_now) * low_env
                + self.upper_gain * sine(upper_now) * upper_env
                + click
                + snap)
    }
}
