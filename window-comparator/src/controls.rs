//! UI state and arithmetic without host callbacks or raw pointers.
#![forbid(unsafe_code)]

use distingnt_api::*;

const LONG_PRESS_MS: u32 = 250;
const POT_RELEASE_BLOCK_MS: u32 = 100;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum EditMode {
    Bounds,
    CenterWidth,
}
impl EditMode {
    pub const fn is_bounds(self) -> bool {
        matches!(self, Self::Bounds)
    }
    pub const fn from_bounds(bounds: bool) -> Self {
        if bounds {
            Self::Bounds
        } else {
            Self::CenterWidth
        }
    }
    pub fn toggle(&mut self) {
        *self = Self::from_bounds(!self.is_bounds());
    }
}

#[derive(Clone, Copy)]
pub(super) enum Pot {
    Left,
    Right,
}

#[derive(Clone, Copy, Default)]
struct Button {
    down_since: u32,
    long_fired: bool,
}
impl Button {
    fn push(&mut self, now: u32) {
        self.down_since = now;
    }
    fn release(&mut self, now: u32) -> bool {
        if self.down_since == 0 {
            return false;
        }
        let short = now.wrapping_sub(self.down_since) < LONG_PRESS_MS;
        if !short {
            self.long_fired = false;
        }
        self.down_since = 0;
        short
    }
    fn poll_long(&mut self, now: u32) -> bool {
        if self.down_since > 0
            && !self.long_fired
            && now.wrapping_sub(self.down_since) >= LONG_PRESS_MS
        {
            self.long_fired = true;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Default)]
struct PotState {
    down_since: u32,
    block_until: u32,
    previous: f32,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Controls {
    pots: [PotState; 2],
    encoder_button: Button,
    has_previous: bool,
}

#[derive(Clone, Copy)]
pub(super) struct PotMovement {
    current: f32,
    previous: Option<f32>,
}
impl PotMovement {
    /// Relative soft takeover, including the original first-move and jump guards.
    pub fn apply(self, value: f32, min: f32, max: f32) -> f32 {
        let Some(previous) = self.previous else {
            return value;
        };
        let delta = self.current - previous;
        if !(delta.abs() <= 0.01) {
            return value;
        }
        let increasing = self.current > previous;
        if increasing && self.current > 0.995 {
            return max;
        }
        if !increasing && self.current < 0.005 {
            return min;
        }
        let range = if increasing { max - value } else { value - min };
        let pot_range = if increasing { 1.0 - previous } else { previous };
        value + range / pot_range * delta
    }
}

impl Controls {
    pub const fn new() -> Self {
        const POT: PotState = PotState {
            down_since: 0,
            block_until: 0,
            previous: 0.0,
        };
        Self {
            pots: [POT; 2],
            encoder_button: Button {
                down_since: 0,
                long_fired: false,
            },
            has_previous: false,
        }
    }
    pub fn movement(&self, pot: Pot, data: &NtUiData, now: u32) -> Option<PotMovement> {
        let (state, mask, current) = match pot {
            Pot::Left => (&self.pots[0], POT_L, data.pots[0]),
            Pot::Right => (&self.pots[1], POT_R, data.pots[2]),
        };
        if data.controls as u32 & mask == 0 || state.down_since != 0 || state.block_until > now {
            return None;
        }
        Some(PotMovement {
            current,
            previous: if self.has_previous {
                Some(state.previous)
            } else {
                None
            },
        })
    }
    /// Must run after pot turns, matching ViewBase's event ordering.
    /// Returns whether the right encoder's short press toggles the edit lock.
    pub fn update_buttons(&mut self, data: &NtUiData, now: u32) -> bool {
        let current = data.controls as u32;
        let last = data.last_buttons as u32;
        if current & ENCODER_BUTTON_R != 0 && last & ENCODER_BUTTON_R == 0 {
            self.encoder_button.push(now);
        }
        let toggle_lock = if current & ENCODER_BUTTON_R == 0 && last & ENCODER_BUTTON_R != 0 {
            self.encoder_button.release(now)
        } else {
            false
        };
        for (pot, mask) in self.pots.iter_mut().zip([POT_BUTTON_L, POT_BUTTON_R]) {
            if current & mask != 0 && last & mask == 0 {
                pot.down_since = now;
            } else if current & mask == 0 && last & mask != 0 && pot.down_since > 0 {
                pot.down_since = 0;
                pot.block_until = now.wrapping_add(POT_RELEASE_BLOCK_MS);
            }
        }
        toggle_lock
    }
    pub fn record_pots(&mut self, values: [f32; 3]) {
        for (pot, value) in self.pots.iter_mut().zip([values[0], values[2]]) {
            pot.previous = value;
        }
        self.has_previous = true;
    }
    /// Called during drawing, just as long presses were processed in ViewBase.
    pub fn poll_long_presses(&mut self, now: u32) -> bool {
        self.encoder_button.poll_long(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(current: u32, previous: u32, left: f32) -> NtUiData {
        NtUiData {
            pots: [left, 0.5, 0.5],
            controls: current as u16,
            last_buttons: previous as u16,
            encoders: [0; 2],
            unused: [0; 2],
        }
    }

    #[test]
    fn press_threshold_is_exact_and_long_press_fires_once() {
        let mut controls = Controls::new();
        assert!(!controls.update_buttons(&input(ENCODER_BUTTON_R, 0, 0.5), 100));
        assert!(!controls.poll_long_presses(349));
        assert!(controls.update_buttons(&input(0, ENCODER_BUTTON_R, 0.5), 349));

        controls.update_buttons(&input(ENCODER_BUTTON_R, 0, 0.5), 400);
        assert!(controls.poll_long_presses(650));
        assert!(!controls.poll_long_presses(900));
        assert!(!controls.update_buttons(&input(0, ENCODER_BUTTON_R, 0.5), 901));
        controls.update_buttons(&input(ENCODER_BUTTON_R, 0, 0.5), 1000);
        assert!(controls.poll_long_presses(1250));
    }

    #[test]
    fn pot_takeover_primes_ignores_jumps_and_respects_release_block() {
        let mut controls = Controls::new();
        let move_data = input(POT_L, 0, 0.505);
        let movement = controls.movement(Pot::Left, &move_data, 10).unwrap();
        assert_eq!(movement.apply(1.0, -5.0, 5.0), 1.0);
        controls.record_pots([0.5; 3]);
        let movement = controls.movement(Pot::Left, &move_data, 10).unwrap();
        assert!((movement.apply(1.0, -5.0, 5.0) - 1.04).abs() < 0.00001);
        let movement = controls
            .movement(Pot::Left, &input(POT_L, 0, 0.7), 10)
            .unwrap();
        assert_eq!(movement.apply(1.0, -5.0, 5.0), 1.0);

        controls.update_buttons(&input(POT_BUTTON_L, 0, 0.5), 20);
        assert!(controls.movement(Pot::Left, &move_data, 30).is_none());
        controls.update_buttons(&input(0, POT_BUTTON_L, 0.5), 40);
        assert!(controls.movement(Pot::Left, &move_data, 139).is_none());
        assert!(controls.movement(Pot::Left, &move_data, 140).is_some());

        controls.record_pots([0.99; 3]);
        let movement = controls
            .movement(Pot::Left, &input(POT_L, 0, 0.996), 150)
            .unwrap();
        assert_eq!(movement.apply(1.0, -5.0, 5.0), 5.0);
        controls.record_pots([0.01; 3]);
        let movement = controls
            .movement(Pot::Right, &input(POT_R, 0, 0.0), 150)
            .unwrap();
        // The right pot is unchanged by input(), so this is an ignored jump.
        assert_eq!(movement.apply(1.0, -5.0, 5.0), 1.0);
    }
}
