//! Output selection and cached, smoothly changing equal-power pan gains.
#![forbid(unsafe_code)]
use super::{levels, LAST_BUS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Routing {
    pub left: i16,
    pub right: i16,
}
impl Routing {
    pub fn new(left: i16, right: i16) -> Self {
        let valid = |bus| {
            if (1..=LAST_BUS as i16).contains(&bus) {
                bus
            } else {
                0
            }
        };
        Self {
            left: valid(left),
            right: valid(right),
        }
    }
    pub fn stereo(self) -> bool {
        self.left != 0 && self.right != 0 && self.left != self.right
    }
    pub fn enabled(self) -> bool {
        self.left != 0 || self.right != 0
    }
    pub fn primary(self) -> i16 {
        if self.left != 0 {
            self.left
        } else {
            self.right
        }
    }
    pub fn sample_on(self, bus: i16, first: f32, second: f32) -> f32 {
        if bus == 0 {
            0.0
        } else if self.stereo() {
            if bus == self.left {
                first
            } else if bus == self.right {
                second
            } else {
                0.0
            }
        } else if bus == self.primary() {
            first
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy)]
pub struct Pan {
    position: i16,
    moving: bool,
    stereo: bool,
    left: f32,
    right: f32,
    target_left: f32,
    target_right: f32,
}
impl Pan {
    pub const fn new() -> Self {
        Self {
            position: 50,
            moving: false,
            stereo: true,
            left: core::f32::consts::FRAC_1_SQRT_2,
            right: core::f32::consts::FRAC_1_SQRT_2,
            target_left: core::f32::consts::FRAC_1_SQRT_2,
            target_right: core::f32::consts::FRAC_1_SQRT_2,
        }
    }
    pub fn prepare(&mut self, position: i16, immediate: bool) {
        let position = position.clamp(0, 100);
        if position != self.position || !self.stereo {
            self.position = position;
            let pan = position as f32 * 0.01;
            self.target_left = levels::sqrt(1.0 - pan);
            self.target_right = levels::sqrt(pan);
            self.moving = true;
        }
        self.stereo = true;
        if immediate {
            self.left = self.target_left;
            self.right = self.target_right;
            self.moving = false;
        }
    }
    pub fn gains(&self) -> (f32, f32) {
        (self.left, self.right)
    }
    pub fn moving(&self) -> bool {
        self.moving
    }
    pub fn start_from(&mut self, left: f32, right: f32) {
        self.left = left;
        self.right = right;
        self.moving = self.left != self.target_left || self.right != self.target_right;
    }
    pub fn mono(&mut self, immediate: bool) {
        self.stereo = false;
        self.target_left = 1.0;
        self.target_right = 0.0;
        if immediate {
            self.left = 1.0;
            self.right = 0.0;
            self.moving = false;
        } else {
            self.moving = self.left != 1.0 || self.right != 0.0;
        }
    }
    /// Return actual individual stereo energy for the linked RMS reference.
    pub fn mix(&mut self, voice: &[f32], left: &mut [f32], right: &mut [f32], slew: f32) -> f32 {
        if !self.moving {
            let count = voice.len().min(left.len()).min(right.len());
            let full = count & !1;
            let (left_gain, right_gain) = (self.left, self.right);
            let mut power = 0.0;
            if let (Some(v), Some(l), Some(r)) = (
                voice.get(..full),
                left.get_mut(..full),
                right.get_mut(..full),
            ) {
                for ((v, l), r) in v
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .zip(l.as_chunks_mut::<2>().0.iter_mut())
                    .zip(r.as_chunks_mut::<2>().0.iter_mut())
                {
                    macro_rules! mix_sample {
                        ($i:expr) => {{
                            let a = v[$i] * left_gain;
                            let b = v[$i] * right_gain;
                            l[$i] += a;
                            r[$i] += b;
                            power += a * a + b * b;
                        }};
                    }
                    mix_sample!(0);
                    mix_sample!(1);
                }
            }
            if let (Some(v), Some(l), Some(r)) = (
                voice.get(full..count),
                left.get_mut(full..count),
                right.get_mut(full..count),
            ) {
                for ((&sample, l), r) in v.iter().zip(l).zip(r) {
                    let a = sample * left_gain;
                    let b = sample * right_gain;
                    *l += a;
                    *r += b;
                    power += a * a + b * b;
                }
            }
            return power;
        }
        let mut power = 0.0;
        for ((&sample, left), right) in voice.iter().zip(left).zip(right) {
            let l = sample * self.left;
            let r = sample * self.right;
            *left += l;
            *right += r;
            power += l * l + r * r;
            if self.moving {
                let previous = (self.left, self.right);
                self.left += slew * (self.target_left - self.left);
                self.right += slew * (self.target_right - self.right);
                if (self.left - self.target_left).abs() < 0.000002
                    && (self.right - self.target_right).abs() < 0.000002
                    || (self.left, self.right) == previous
                {
                    self.left = self.target_left;
                    self.right = self.target_right;
                    self.moving = false;
                }
            }
        }
        power
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reference_mix(
        pan: &mut Pan,
        voice: &[f32],
        left: &mut [f32],
        right: &mut [f32],
        slew: f32,
    ) -> f32 {
        let mut power = 0.0;
        for ((&sample, left), right) in voice.iter().zip(left).zip(right) {
            let l = sample * pan.left;
            let r = sample * pan.right;
            *left += l;
            *right += r;
            power += l * l + r * r;
            if pan.moving {
                let previous = (pan.left, pan.right);
                pan.left += slew * (pan.target_left - pan.left);
                pan.right += slew * (pan.target_right - pan.right);
                if (pan.left - pan.target_left).abs() < 0.000002
                    && (pan.right - pan.target_right).abs() < 0.000002
                    || (pan.left, pan.right) == previous
                {
                    pan.left = pan.target_left;
                    pan.right = pan.target_right;
                    pan.moving = false;
                }
            }
        }
        power
    }
    #[test]
    fn routing_infers_mono_stereo_and_duplicate_buses() {
        for (left, right, enabled, stereo, primary) in [
            (0, 0, false, false, 0),
            (13, 0, true, false, 13),
            (0, 14, true, false, 14),
            (13, 14, true, true, 13),
            (13, 13, true, false, 13),
            (-1, 14, true, false, 14),
        ] {
            let routing = Routing::new(left, right);
            assert_eq!(
                (routing.enabled(), routing.stereo(), routing.primary()),
                (enabled, stereo, primary)
            );
        }
    }
    #[test]
    fn pan_endpoints_center_and_total_energy_are_correct() {
        let mut pan = Pan::new();
        for position in 0..=100 {
            pan.prepare(position, true);
            let mut left = [0.0; 16];
            let mut right = [0.0; 16];
            let energy = pan.mix(&[1.0; 16], &mut left, &mut right, 1000.0 / 48000.0);
            assert!((energy - 16.0).abs() < 0.00001);
            assert!((left[0] * left[0] - (1.0 - position as f32 * 0.01)).abs() < 0.000001);
            if position == 0 {
                assert_eq!(right, [0.0; 16]);
            }
            if position == 100 {
                assert_eq!(left, [0.0; 16]);
            }
        }
    }
    #[test]
    fn pan_changes_smoothly_and_reaches_the_requested_endpoint() {
        let mut pan = Pan::new();
        pan.prepare(0, true);
        pan.prepare(100, false);
        let mut left = [0.0; 16];
        let mut right = [0.0; 16];
        pan.mix(&[1.0; 16], &mut left, &mut right, 1000.0 / 48000.0);
        assert!(left[0] > 0.9 && right[0] < 0.1);
        for _ in 0..80 {
            pan.mix(&[1.0; 16], &mut [0.0; 16], &mut [0.0; 16], 1000.0 / 48000.0);
        }
        assert_eq!((pan.left, pan.right), (0.0, 1.0));
    }
    #[test]
    fn unrolled_pan_matches_reference_for_all_positions_lengths_and_transitions() {
        for frames in 0..=64 {
            for position in 0..=100 {
                let mut actual = Pan::new();
                actual.prepare(position, true);
                let mut expected = actual;
                let mut voice = [0.0; 64];
                for (i, v) in voice.iter_mut().enumerate() {
                    *v = ((i * 73) % 257) as f32 / 32.0 - 4.0;
                }
                for block in 0..5 {
                    match block {
                        1 => {
                            actual.prepare(100 - position, false);
                            expected.prepare(100 - position, false);
                        }
                        2 => {
                            actual.mono(false);
                            expected.mono(false);
                        }
                        3 => {
                            actual.prepare(position, true);
                            expected.prepare(position, true);
                        }
                        _ => {}
                    }
                    let mut left = [0.375; 65];
                    let mut right = [-0.25; 66];
                    let mut el = left;
                    let mut er = right;
                    let power = actual.mix(
                        &voice[..frames],
                        &mut left[..frames + 1],
                        &mut right[..frames + 2],
                        0.05,
                    );
                    let ep = reference_mix(
                        &mut expected,
                        &voice[..frames],
                        &mut el[..frames + 1],
                        &mut er[..frames + 2],
                        0.05,
                    );
                    assert_eq!(power.to_bits(), ep.to_bits());
                    for (a, b) in left.iter().chain(&right).zip(el.iter().chain(&er)) {
                        assert_eq!(a.to_bits(), b.to_bits());
                    }
                    assert_eq!(actual.moving, expected.moving);
                    assert_eq!(actual.gains(), expected.gains());
                }
            }
        }
    }
}
