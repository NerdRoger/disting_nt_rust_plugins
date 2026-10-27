//! Checked views of contiguous host audio buffers. No gate/trigger policy.
#![forbid(unsafe_code)]
use crate::raw::LAST_BUS;
use core::{num::NonZeroUsize, ops::Range};

#[inline]
pub fn bus_index(parameter: i16) -> Option<usize> {
    (1..=LAST_BUS as i16)
        .contains(&parameter)
        .then(|| parameter as usize - 1)
}

pub struct AudioBuses<'a> {
    samples: &'a mut [f32],
    frames: NonZeroUsize,
}

impl<'a> AudioBuses<'a> {
    #[inline]
    pub fn new(samples: &'a mut [f32], frames: usize) -> Option<Self> {
        let frames = NonZeroUsize::new(frames)?;
        if samples.len() != frames.get().checked_mul(LAST_BUS)? {
            return None;
        }
        Some(Self { samples, frames })
    }
    #[inline]
    pub fn frames(&self) -> usize {
        self.frames.get()
    }
    #[inline]
    fn range(&self, parameter: i16) -> Option<Range<usize>> {
        let start = bus_index(parameter)? * self.frames.get();
        Some(start..start + self.frames.get())
    }
    #[inline]
    pub fn bus(&self, parameter: i16) -> Option<&[f32]> {
        self.samples.get(self.range(parameter)?)
    }
    #[inline]
    pub fn bus_mut(&mut self, parameter: i16) -> Option<&mut [f32]> {
        let range = self.range(parameter)?;
        self.samples.get_mut(range)
    }
    #[inline]
    pub fn read_first(&self, parameter: i16) -> f32 {
        self.range(parameter)
            .and_then(|range| self.samples.get(range.start))
            .copied()
            .unwrap_or(0.0)
    }
    #[inline]
    pub fn fill(&mut self, parameter: i16, value: f32) {
        if let Some(samples) = self.bus_mut(parameter) {
            samples.fill(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dimensions_and_bus_bounds_are_checked() {
        let mut samples = [2.0; LAST_BUS * 4];
        assert!(AudioBuses::new(&mut samples, 0).is_none());
        assert!(AudioBuses::new(&mut samples, usize::MAX).is_none());
        assert!(AudioBuses::new(&mut samples, 8).is_none());
        let mut buses = AudioBuses::new(&mut samples, 4).unwrap();
        for index in [-1, 0, 65, i16::MAX] {
            assert!(buses.bus_mut(index).is_none());
        }
        buses.fill(64, -3.0);
        assert_eq!(buses.read_first(64), -3.0);
        assert_eq!(buses.read_first(1), 2.0);
        assert_eq!(buses.frames(), 4);
    }
}
