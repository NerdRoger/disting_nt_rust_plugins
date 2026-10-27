//! Parameter synchronization. Each host operation finishes before the next read.
#![forbid(unsafe_code)]
use super::{cp, engine::Guard, gp, Callback};
use distingnt_api::api::parameters::Audio;

pub(super) fn changed(context: &mut Callback<'_, Audio>, parameter: usize) -> Option<()> {
    match parameter {
        gp::RANGE_MIN | gp::RANGE_MAX => {
            update_global_range(context, parameter)?;
            for channel in 0..context.state().num_channels.get() {
                update_channel_range(context, channel)?;
            }
        }
        _ => {
            for channel in 0..context.state().num_channels.get() {
                update_channel(context, channel, parameter)?;
            }
        }
    }
    Some(())
}

fn update_global_range(context: &mut Callback<'_, Audio>, parameter: usize) -> Option<()> {
    let minimum_changed = parameter == gp::RANGE_MIN;
    let scaled = context.read_parameters().scaled(parameter)? as i8;
    if minimum_changed {
        context.state_mut().range_min = scaled;
    } else {
        context.state_mut().range_max = scaled;
    }
    let value = context.read_parameters().value(parameter)?;
    let opposite = if minimum_changed {
        gp::RANGE_MAX
    } else {
        gp::RANGE_MIN
    };
    context
        .parameters()
        .update_definition(opposite, |definition| {
            if minimum_changed {
                definition.min = value;
            } else {
                definition.max = value;
            }
        })?;
    // Re-read after notifying: a synchronous callback may have changed the range.
    let state = context.state();
    let width = range_width(state.range_min, state.range_max);
    context.state_mut().range = width;
    Some(())
}

fn update_channel_range(context: &mut Callback<'_, Audio>, channel: usize) -> Option<()> {
    let offset = context.voices().get(channel)?.offset();
    // Keep this read inside the channel loop, matching reentrant notification order.
    let limits = Limits::from_range(
        context.read_parameters().scaled(gp::RANGE_MIN)? as i8,
        context.read_parameters().scaled(gp::RANGE_MAX)? as i8,
    );
    context
        .parameters()
        .update_definition(offset + cp::LEFT, |definition| definition.min = limits.min)?;
    context
        .parameters()
        .update_definition(offset + cp::RIGHT, |definition| definition.max = limits.max)?;
    context
        .parameters()
        .update_definition(offset + cp::CENTER, |definition| {
            definition.min = limits.min;
            definition.max = limits.max;
        })?;
    context
        .parameters()
        .update_definition(offset + cp::WIDTH, |definition| {
            definition.max = limits.max_width
        })?;
    Some(())
}

// The closure borrows the context only while running. No state/voice reference
// spans a setter, and cleanup runs even if the operation returns None.
fn with_guard(
    context: &mut Callback<'_, Audio>,
    channel: usize,
    guard: Guard,
    operation: impl FnOnce(&mut Callback<'_, Audio>) -> Option<()>,
) -> Option<()> {
    context
        .voices_mut()
        .get_mut(channel)?
        .set_guard(guard, true);
    let result = operation(context);
    context
        .voices_mut()
        .get_mut(channel)?
        .set_guard(guard, false);
    result
}

// Sharing this dispatcher is smaller on ARM than inlining it into the factory callback.
#[inline(never)]
fn update_channel(
    context: &mut Callback<'_, Audio>,
    channel: usize,
    parameter: usize,
) -> Option<()> {
    let offset = context.voices().get(channel)?.offset();
    match parameter.checked_sub(offset) {
        Some(cp::OVERRIDE_ATTENUATION) => {
            let gray = context.read_parameters().value(parameter)? != 1;
            context.parameters().gray(offset + cp::INPUT_SCALE, gray)?;
            context.parameters().gray(offset + cp::INPUT_OFFSET, gray)
        }
        Some(cp::LEFT | cp::RIGHT) => with_guard(context, channel, Guard::Bounds, |context| {
            synchronize_bounds(context, channel, offset, parameter == offset + cp::LEFT)
        }),
        Some(cp::CENTER | cp::WIDTH) => {
            with_guard(context, channel, Guard::SizePosition, |context| {
                synchronize_center_width(context, channel, offset)
            })
        }
        _ => Some(()),
    }
}

fn synchronize_bounds(
    context: &mut Callback<'_, Audio>,
    channel: usize,
    offset: usize,
    left_changed: bool,
) -> Option<()> {
    let values = center_width(
        context.read_parameters().value(offset + cp::LEFT)?,
        context.read_parameters().value(offset + cp::RIGHT)?,
    );
    if !context.voices().get(channel)?.guard(Guard::SizePosition) {
        context
            .parameters()
            .set(offset + cp::CENTER, values.center as i16)?;
        context
            .parameters()
            .set(offset + cp::WIDTH, values.width as i16)?;
    }
    // Read after setters so nested updates are retained.
    if left_changed {
        let min = context.read_parameters().value(offset + cp::LEFT)?;
        context
            .parameters()
            .update_definition(offset + cp::RIGHT, |definition| definition.min = min)?;
    } else {
        let max = context.read_parameters().value(offset + cp::RIGHT)?;
        context
            .parameters()
            .update_definition(offset + cp::LEFT, |definition| definition.max = max)?;
    }
    Some(())
}

fn synchronize_center_width(
    context: &mut Callback<'_, Audio>,
    channel: usize,
    offset: usize,
) -> Option<()> {
    let values = bounds(
        context.read_parameters().value(offset + cp::CENTER)?,
        context.read_parameters().value(offset + cp::WIDTH)?,
    );
    if !context.voices().get(channel)?.guard(Guard::Bounds) {
        context
            .parameters()
            .set(offset + cp::LEFT, values.left as i16)?;
        context
            .parameters()
            .set(offset + cp::RIGHT, values.right as i16)?;
    }
    Some(())
}

#[derive(Debug, PartialEq)]
pub(super) struct CenterWidth {
    pub center: i32,
    pub width: i32,
}

#[derive(Debug, PartialEq)]
pub(super) struct Bounds {
    pub left: i32,
    pub right: i32,
}

pub(super) fn center_width(left: i16, right: i16) -> CenterWidth {
    // C++ promotes parameter values to int before arithmetic. Keep its
    // truncation toward zero when the midpoint lies between parameter units.
    let left = i32::from(left);
    let right = i32::from(right);
    CenterWidth {
        center: (right + left) / 2,
        width: right - left,
    }
}

pub(super) fn bounds(center: i16, width: i16) -> Bounds {
    let center = i32::from(center);
    let half_width = i32::from(width) / 2;
    Bounds {
        left: center - half_width,
        right: center + half_width,
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct Limits {
    pub min: i16,
    pub max: i16,
    pub max_width: i16,
}
impl Limits {
    pub fn from_range(min: i8, max: i8) -> Self {
        Self {
            min: (i32::from(min) * 1000) as i16,
            max: (i32::from(max) * 1000) as i16,
            max_width: ((i32::from(max) - i32::from(min)) * 1000) as i16,
        }
    }
}

pub(super) fn range_width(min: i8, max: i8) -> u8 {
    (i16::from(max) - i16::from(min)) as u8
}

#[cfg(test)]
mod arithmetic_tests {
    use super::*;

    #[test]
    fn midpoint_rounding_and_odd_widths_match_cpp() {
        assert_eq!(
            center_width(-1000, 1001),
            CenterWidth {
                center: 0,
                width: 2001
            }
        );
        assert_eq!(
            center_width(-1001, 1000),
            CenterWidth {
                center: 0,
                width: 2001
            }
        );
        assert_eq!(
            center_width(-1001, -1000),
            CenterWidth {
                center: -1000,
                width: 1
            }
        );
        assert_eq!(
            bounds(-1000, 3),
            Bounds {
                left: -1001,
                right: -999
            }
        );
        assert_eq!(
            bounds(500, 0),
            Bounds {
                left: 500,
                right: 500
            }
        );
        assert_eq!(
            bounds(0, 20000),
            Bounds {
                left: -10000,
                right: 10000
            }
        );
    }

    #[test]
    fn range_limits_cover_supported_extremes_and_collapsed_ranges() {
        assert_eq!(
            Limits::from_range(-10, 10),
            Limits {
                min: -10000,
                max: 10000,
                max_width: 20000
            }
        );
        assert_eq!(
            Limits::from_range(3, 3),
            Limits {
                min: 3000,
                max: 3000,
                max_width: 0
            }
        );
        assert_eq!(range_width(-10, 10), 20);
        assert_eq!(range_width(3, 3), 0);
    }
}
