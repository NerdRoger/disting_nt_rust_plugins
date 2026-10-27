//! Sequential audio processing, comparator state, and bus output policy.
#![forbid(unsafe_code)]
use super::{
    agg, channel_offset, count_parameters, cp, gate, gp, Callback, CHANNEL_PARAMS, COMMON_AGG,
    LAST_BUS,
};
use distingnt_api::api::{
    audio::{bus_index, AudioBuses},
    parameters::Audio,
};
const BUS_COUNT: usize = LAST_BUS;

#[derive(Clone, Copy)]
pub(super) enum Guard {
    Bounds,
    SizePosition,
}

#[derive(Clone, Copy)]
pub(super) struct Input {
    pub sample: f32,
    pub scale: f32,
    pub offset: f32,
    pub range_min: f32,
    pub range_max: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Comparator {
    previously_inside: bool,
    pub current_value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Comparison {
    pub inside: bool,
    pub entered: bool,
    pub exited: bool,
}

impl Comparator {
    pub fn process(&mut self, input: Input, first_step: bool) -> Comparison {
        let value = input.sample * input.scale / 100.0 + input.offset;
        // Match C++ comparisons even for NaN or a zero-width range; f32::clamp
        // has different preconditions and NaN handling.
        self.current_value = if value < input.range_min {
            input.range_min
        } else if value > input.range_max {
            input.range_max
        } else {
            value
        };
        let inside = self.current_value >= input.left && self.current_value <= input.right;
        let result = Comparison {
            inside,
            entered: inside && !self.previously_inside && !first_step,
            exited: !inside && self.previously_inside && !first_step,
        };
        self.previously_inside = inside;
        result
    }
}

#[derive(Default)]
pub(super) struct Aggregates {
    pub inside_count: usize,
    pub outside_count: usize,
    inside_total: [f32; 3],
    outside_total: [f32; 3],
}

impl Aggregates {
    pub fn add(&mut self, inside: bool, contributions: [f32; 3]) {
        let totals = if inside {
            self.inside_count += 1;
            &mut self.inside_total
        } else {
            self.outside_count += 1;
            &mut self.outside_total
        };
        for (total, contribution) in totals.iter_mut().zip(contributions) {
            *total += contribution;
        }
    }

    /// Four outputs per axis, preserving parameter order and IEEE empty-set NaN.
    pub fn values(&self) -> impl Iterator<Item = [f32; 4]> + '_ {
        self.inside_total
            .iter()
            .zip(&self.outside_total)
            .map(|(&inside, &outside)| {
                [
                    inside,
                    outside,
                    inside / self.inside_count as f32,
                    outside / self.outside_count as f32,
                ]
            })
    }
}

#[derive(Clone, Copy)]
pub(super) struct Channel {
    offset: usize,
    comparator: Comparator,
    updating_bounds: bool,
    updating_size_pos: bool,
}

impl Channel {
    pub fn new(index: usize) -> Option<Self> {
        if index >= 8 {
            return None;
        }
        Some(Self {
            offset: channel_offset(index),
            comparator: Comparator::default(),
            updating_bounds: false,
            updating_size_pos: false,
        })
    }
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub fn compare(&mut self, input: Input, first_step: bool) -> Comparison {
        self.comparator.process(input, first_step)
    }
    pub fn current_value(&self) -> f32 {
        self.comparator.current_value
    }
    pub fn guard(&self, guard: Guard) -> bool {
        match guard {
            Guard::Bounds => self.updating_bounds,
            Guard::SizePosition => self.updating_size_pos,
        }
    }
    pub fn set_guard(&mut self, guard: Guard, value: bool) {
        match guard {
            Guard::Bounds => self.updating_bounds = value,
            Guard::SizePosition => self.updating_size_pos = value,
        }
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;
    #[test]
    fn channel_offsets_and_recursion_flags_are_independent() {
        assert!(Channel::new(8).is_none());
        assert!(Channel::new(usize::MAX).is_none());
        for index in 0..8 {
            let mut channel = Channel::new(index).unwrap();
            assert_eq!(channel.offset(), channel_offset(index));
            assert!(!channel.guard(Guard::Bounds) && !channel.guard(Guard::SizePosition));
            channel.set_guard(Guard::Bounds, true);
            assert!(channel.guard(Guard::Bounds) && !channel.guard(Guard::SizePosition));
            channel.set_guard(Guard::SizePosition, true);
            channel.set_guard(Guard::Bounds, false);
            assert!(!channel.guard(Guard::Bounds) && channel.guard(Guard::SizePosition));
        }
    }
}

struct AudioBlock<'a> {
    buses: AudioBuses<'a>,
    cleared: [bool; BUS_COUNT],
}
impl<'a> AudioBlock<'a> {
    pub fn new(samples: &'a mut [f32], frames: usize) -> Option<Self> {
        Some(Self {
            buses: AudioBuses::new(samples, frames)?,
            cleared: [false; BUS_COUNT],
        })
    }
    pub fn read_first(&self, parameter: i16) -> f32 {
        self.buses.read_first(parameter)
    }
    pub fn event(&mut self, parameter: i16, high: bool) {
        let Some(bus) = bus_index(parameter) else {
            return;
        };
        let Some(out) = self.buses.bus_mut(parameter) else {
            return;
        };
        let Some(cleared) = self.cleared.get_mut(bus) else {
            return;
        };
        if !*cleared {
            out.fill(0.0);
            *cleared = true;
        }
        if high {
            out.fill(10.0);
        }
    }
    pub fn write_value(&mut self, parameter: i16, value: f32) {
        self.buses.fill(parameter, value);
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;

    #[test]
    fn disabled_and_invalid_buses_do_not_touch_samples() {
        let mut samples = [2.0; BUS_COUNT * 4];
        let mut block = AudioBlock::new(&mut samples, 4).unwrap();
        for bus in [-1, 0, (BUS_COUNT + 1) as i16, i16::MAX] {
            assert_eq!(block.read_first(bus), 0.0);
            block.event(bus, true);
            block.write_value(bus, 5.0);
        }
        assert!(samples.iter().all(|&v| v == 2.0));
    }

    #[test]
    fn shared_bus_writes_keep_their_order() {
        let mut samples = [2.0; BUS_COUNT * 4];
        let mut block = AudioBlock::new(&mut samples, 4).unwrap();
        block.write_value(2, 3.0);
        block.event(2, false);
        assert_eq!(block.read_first(2), 0.0); // first event clears earlier values
        block.event(2, true);
        block.event(2, false);
        assert_eq!(block.read_first(2), 10.0); // low events never pull a high down
        block.write_value(2, 7.0);
        block.event(2, false);
        assert_eq!(block.read_first(2), 7.0); // clear only once per bus per block
        block.write_value(BUS_COUNT as i16, -4.0);
        assert_eq!(block.read_first(BUS_COUNT as i16), -4.0);
        assert_eq!(block.read_first(1), 2.0);
        assert_eq!(&samples[4..8], &[7.0; 4]);
        assert_eq!(&samples[(BUS_COUNT - 1) * 4..], &[-4.0; 4]);
    }

    #[test]
    fn block_dimensions_must_match_the_host_layout() {
        let mut samples = [0.0; BUS_COUNT * 4];
        assert!(AudioBlock::new(&mut samples, 0).is_none());
        assert!(AudioBlock::new(&mut samples, 8).is_none());
        assert!(AudioBlock::new(&mut samples, usize::MAX).is_none());
        assert!(AudioBlock::new(&mut samples, 4).is_some());
    }
}

// Events remain high for their current processing block, matching C++.
pub(super) fn step(
    context: &mut Callback<'_, Audio>,
    samples: &mut [f32],
    frames: usize,
) -> Option<()> {
    let mut buses = AudioBlock::new(samples, frames)?;
    let data = context.processing();
    let parameters = data.parameters;
    let channels = data.voices;
    if parameters.len() != count_parameters(channels.len()) {
        return None;
    }
    let first_step = data.state.first_step;
    let range_min = data.state.range_min;
    let range_max = data.state.range_max;
    let mut totals = Aggregates::default();
    let global_scale = parameters.scaled(gp::INPUT_SCALE)?;
    let global_offset = parameters.scaled(gp::INPUT_OFFSET)?;
    for state in channels.iter_mut() {
        let o = state.offset();
        let local = parameters.group::<{ cp::COUNT }>(o)?;
        let input = local.value(cp::INPUT)?;
        let over = local.value(cp::OVERRIDE_ATTENUATION)? == 1;
        let scale = if over {
            local.scaled(cp::INPUT_SCALE)?
        } else {
            global_scale
        };
        let offset = if over {
            local.scaled(cp::INPUT_OFFSET)?
        } else {
            global_offset
        };
        let raw = buses.read_first(input);
        let input = Input {
            sample: raw,
            scale,
            offset,
            range_min: range_min as f32,
            range_max: range_max as f32,
            left: local.scaled(cp::LEFT)?,
            right: local.scaled(cp::RIGHT)?,
        };
        let result = state.compare(input, first_step);
        totals.add(
            result.inside,
            [
                local.scaled(cp::VALUE_X)?,
                local.scaled(cp::VALUE_Y)?,
                local.scaled(cp::VALUE_Z)?,
            ],
        );
        for (p, high) in [
            (cp::INSIDE_GATE, result.inside),
            (cp::OUTSIDE_GATE, !result.inside),
            (cp::ENTER_TRIGGER, result.entered),
            (cp::EXIT_TRIGGER, result.exited),
        ] {
            buses.event(local.value(p)?, high);
        }
    }
    if channels.len() > 1 {
        let aggregate = parameters.group::<{ agg::COUNT }>(COMMON_AGG)?;
        buses.event(
            aggregate.value(agg::ALL_INSIDE_GATE)?,
            totals.inside_count == channels.len(),
        );
        buses.event(
            aggregate.value(agg::ALL_OUTSIDE_GATE)?,
            totals.outside_count == channels.len(),
        );
        for (axis, values) in totals.values().enumerate() {
            for (j, output) in values.into_iter().enumerate() {
                buses.write_value(aggregate.value(axis * 4 + j)?, output);
            }
        }
    }
    for (ch, state) in channels.iter().enumerate().skip(1) {
        let o = state.offset() + CHANNEL_PARAMS;
        let gates = parameters.group::<{ gate::COUNT }>(o)?;
        for (j, high) in [
            (gate::EXACTLY_INSIDE, totals.inside_count == ch),
            (gate::EXACTLY_OUTSIDE, totals.outside_count == ch),
            (gate::AT_LEAST_INSIDE, totals.inside_count >= ch),
            (gate::AT_LEAST_OUTSIDE, totals.outside_count >= ch),
        ]
        .into_iter()
        {
            buses.event(gates.value(j)?, high);
        }
    }

    data.state.clock.advance(frames as u32);
    data.state.first_step = false;
    Some(())
}
