//! Validated metadata, stable ordinals, page construction, and parameter text.
#![forbid(unsafe_code)]
use super::*;
use core::ffi::{c_char, CStr};
use distingnt_api::api::metadata::{Definition, Name};
use distingnt_api::api::plugin::Metadata;

static YES_NO: distingnt_api::api::metadata::EnumStrings<2> =
    distingnt_api::api::metadata::EnumStrings::new([c"No", c"Yes"]);

static CHANNEL_NAMES: [Name; 8] = [
    Name::new(c"Channel A"),
    Name::new(c"Channel B"),
    Name::new(c"Channel C"),
    Name::new(c"Channel D"),
    Name::new(c"Channel E"),
    Name::new(c"Channel F"),
    Name::new(c"Channel G"),
    Name::new(c"Channel H"),
];
static AGG_NAMES: [Name; 14] = [
    Name::new(c"Value X Total Inside"),
    Name::new(c"Value X Total Outside"),
    Name::new(c"Value X Avg Inside"),
    Name::new(c"Value X Avg Outside"),
    Name::new(c"Value Y Total Inside"),
    Name::new(c"Value Y Total Outside"),
    Name::new(c"Value Y Avg Inside"),
    Name::new(c"Value Y Avg Outside"),
    Name::new(c"Value Z Total Inside"),
    Name::new(c"Value Z Total Outside"),
    Name::new(c"Value Z Avg Inside"),
    Name::new(c"Value Z Avg Outside"),
    Name::new(c"All Inside Gate"),
    Name::new(c"All Outside Gate"),
];
static COUNTED_NAMES: [[Name; 4]; 7] = [
    [
        Name::new(c"Exactly 1 Inside Gate"),
        Name::new(c"Exactly 1 Outside Gate"),
        Name::new(c"At Least 1 Inside Gate"),
        Name::new(c"At Least 1 Outside Gate"),
    ],
    [
        Name::new(c"Exactly 2 Inside Gate"),
        Name::new(c"Exactly 2 Outside Gate"),
        Name::new(c"At Least 2 Inside Gate"),
        Name::new(c"At Least 2 Outside Gate"),
    ],
    [
        Name::new(c"Exactly 3 Inside Gate"),
        Name::new(c"Exactly 3 Outside Gate"),
        Name::new(c"At Least 3 Inside Gate"),
        Name::new(c"At Least 3 Outside Gate"),
    ],
    [
        Name::new(c"Exactly 4 Inside Gate"),
        Name::new(c"Exactly 4 Outside Gate"),
        Name::new(c"At Least 4 Inside Gate"),
        Name::new(c"At Least 4 Outside Gate"),
    ],
    [
        Name::new(c"Exactly 5 Inside Gate"),
        Name::new(c"Exactly 5 Outside Gate"),
        Name::new(c"At Least 5 Inside Gate"),
        Name::new(c"At Least 5 Outside Gate"),
    ],
    [
        Name::new(c"Exactly 6 Inside Gate"),
        Name::new(c"Exactly 6 Outside Gate"),
        Name::new(c"At Least 6 Inside Gate"),
        Name::new(c"At Least 6 Outside Gate"),
    ],
    [
        Name::new(c"Exactly 7 Inside Gate"),
        Name::new(c"Exactly 7 Outside Gate"),
        Name::new(c"At Least 7 Inside Gate"),
        Name::new(c"At Least 7 Outside Gate"),
    ],
];

#[inline]
const fn param(
    name: &'static CStr,
    min: i16,
    max: i16,
    default: i16,
    unit: u8,
    scaling: u8,
) -> Definition {
    match Definition::new(name, min, max, default, unit, scaling, None) {
        Some(definition) => definition,
        None => panic!("invalid static parameter definition"),
    }
}
static GLOBAL: [Definition; gp::COUNT] = [
    param(c"Range Min", -10, 10, -5, UNIT_CONFIRM, SCALING_NONE),
    param(c"Range Max", -10, 10, 5, UNIT_CONFIRM, SCALING_NONE),
    param(
        c"Atten Input",
        -20000,
        20000,
        10000,
        UNIT_PERCENT,
        SCALING_100,
    ),
    param(c"Offset Input", -1000, 1000, 0, UNIT_VOLTS, SCALING_100),
];
const OUTPUT: Definition = param(c"", 0, LAST_BUS as i16, 0, UNIT_CV_OUTPUT, SCALING_NONE);
static CHANNEL: [Definition; cp::COUNT] = [
    param(c"Input", 0, LAST_BUS as i16, 1, UNIT_CV_INPUT, SCALING_NONE),
    param(c"Window Left", -5000, 5000, -1000, UNIT_VOLTS, SCALING_1000),
    param(c"Window Right", -5000, 5000, 1000, UNIT_VOLTS, SCALING_1000),
    param(c"Window Center", -5000, 5000, 0, UNIT_VOLTS, SCALING_1000),
    param(c"Window Width", 0, 20000, 2000, UNIT_VOLTS, SCALING_1000),
    match Definition::new(
        c"Override Global Atten",
        0,
        1,
        0,
        UNIT_ENUM,
        SCALING_NONE,
        Some(YES_NO.table()),
    ) {
        Some(definition) => definition,
        None => panic!("invalid static enum definition"),
    },
    param(
        c"Atten Input",
        -20000,
        20000,
        10000,
        UNIT_PERCENT,
        SCALING_100,
    ),
    param(c"Offset Input", -1000, 1000, 0, UNIT_VOLTS, SCALING_100),
    param(
        c"Value X Contribution",
        -10000,
        10000,
        1000,
        UNIT_VOLTS,
        SCALING_1000,
    ),
    param(
        c"Value Y Contribution",
        -10000,
        10000,
        500,
        UNIT_VOLTS,
        SCALING_1000,
    ),
    param(
        c"Value Z Contribution",
        -10000,
        10000,
        100,
        UNIT_VOLTS,
        SCALING_1000,
    ),
    param(
        c"Inside Gate",
        0,
        LAST_BUS as i16,
        0,
        UNIT_CV_OUTPUT,
        SCALING_NONE,
    ),
    param(
        c"Outside Gate",
        0,
        LAST_BUS as i16,
        0,
        UNIT_CV_OUTPUT,
        SCALING_NONE,
    ),
    param(
        c"Enter Trig",
        0,
        LAST_BUS as i16,
        0,
        UNIT_CV_OUTPUT,
        SCALING_NONE,
    ),
    param(
        c"Exit Trig",
        0,
        LAST_BUS as i16,
        0,
        UNIT_CV_OUTPUT,
        SCALING_NONE,
    ),
];

// Stable host parameter ordinals; changing these breaks existing presets.
pub(super) mod global {
    pub const RANGE_MIN: usize = 0;
    pub const RANGE_MAX: usize = 1;
    pub const INPUT_SCALE: usize = 2;
    pub const INPUT_OFFSET: usize = 3;
    pub const COUNT: usize = 4;
}

pub(super) mod channel {
    pub const INPUT: usize = 0;
    pub const LEFT: usize = 1;
    pub const RIGHT: usize = 2;
    pub const CENTER: usize = 3;
    pub const WIDTH: usize = 4;
    pub const OVERRIDE_ATTENUATION: usize = 5;
    pub const INPUT_SCALE: usize = 6;
    pub const INPUT_OFFSET: usize = 7;
    pub const VALUE_X: usize = 8;
    pub const VALUE_Y: usize = 9;
    pub const VALUE_Z: usize = 10;
    pub const INSIDE_GATE: usize = 11;
    pub const OUTSIDE_GATE: usize = 12;
    pub const ENTER_TRIGGER: usize = 13;
    pub const EXIT_TRIGGER: usize = 14;
    pub const COUNT: usize = 15;
}

pub(super) mod aggregate {
    pub const X_INSIDE_TOTAL: usize = 0;
    pub const X_OUTSIDE_TOTAL: usize = 1;
    pub const X_INSIDE_AVERAGE: usize = 2;
    pub const X_OUTSIDE_AVERAGE: usize = 3;
    pub const Y_INSIDE_TOTAL: usize = 4;
    pub const Y_OUTSIDE_TOTAL: usize = 5;
    pub const Y_INSIDE_AVERAGE: usize = 6;
    pub const Y_OUTSIDE_AVERAGE: usize = 7;
    pub const Z_INSIDE_TOTAL: usize = 8;
    pub const Z_OUTSIDE_TOTAL: usize = 9;
    pub const Z_INSIDE_AVERAGE: usize = 10;
    pub const Z_OUTSIDE_AVERAGE: usize = 11;
    pub const ALL_INSIDE_GATE: usize = 12;
    pub const ALL_OUTSIDE_GATE: usize = 13;
    pub const COUNT: usize = 14;
}

pub(super) mod counted_gate {
    pub const EXACTLY_INSIDE: usize = 0;
    pub const EXACTLY_OUTSIDE: usize = 1;
    pub const AT_LEAST_INSIDE: usize = 2;
    pub const AT_LEAST_OUTSIDE: usize = 3;
    pub const COUNT: usize = 4;
}

pub(crate) fn build(n: usize, metadata: &mut Metadata<'_>) -> Option<()> {
    if !(1..=8).contains(&n) || metadata.parameter_count() != count_parameters(n) {
        return None;
    }
    for (index, definition) in GLOBAL.iter().copied().enumerate() {
        metadata.set_parameter(index, definition)?;
    }
    let gate_order = [
        gate::EXACTLY_INSIDE,
        gate::EXACTLY_OUTSIDE,
        gate::AT_LEAST_INSIDE,
        gate::AT_LEAST_OUTSIDE,
    ];
    for ch in 0..n {
        let offset = channel_offset(ch);
        for (index, definition) in CHANNEL.iter().copied().enumerate() {
            metadata.set_parameter(offset + index, definition)?;
        }
        if ch > 0 {
            for (index, &name) in COUNTED_NAMES.get(ch - 1)?.iter().enumerate() {
                metadata.set_parameter(offset + cp::COUNT + index, OUTPUT.with_name(name))?;
            }
        }
    }
    if n > 1 {
        for (index, &name) in AGG_NAMES.iter().enumerate() {
            metadata.set_parameter(COMMON_AGG + index, OUTPUT.with_name(name))?;
        }
    }
    metadata.add_page(
        c"Global",
        1,
        &[
            gp::RANGE_MIN as u8,
            gp::RANGE_MAX as u8,
            gp::INPUT_SCALE as u8,
            gp::INPUT_OFFSET as u8,
        ],
    )?;
    for (ch, &name) in CHANNEL_NAMES.iter().take(n).enumerate() {
        let offset = channel_offset(ch) as u8;
        let mut indices = [0; cp::COUNT];
        let order = [
            cp::INPUT,
            cp::LEFT,
            cp::RIGHT,
            cp::CENTER,
            cp::WIDTH,
            cp::OVERRIDE_ATTENUATION,
            cp::INPUT_SCALE,
            cp::INPUT_OFFSET,
            cp::VALUE_X,
            cp::VALUE_Y,
            cp::VALUE_Z,
            cp::INSIDE_GATE,
            cp::OUTSIDE_GATE,
            cp::ENTER_TRIGGER,
            cp::EXIT_TRIGGER,
        ];
        for (index, local) in indices.iter_mut().zip(order) {
            *index = offset + local as u8;
        }
        metadata.add_named_page(name, 2, &indices)?;
    }
    if n > 1 {
        let mut gates = [0; 30];
        let mut count = 0;
        for ch in 1..n {
            let offset = channel_offset(ch) + cp::COUNT;
            for local in gate_order {
                *gates.get_mut(count)? = (offset + local) as u8;
                count += 1;
            }
        }
        *gates.get_mut(count)? = (COMMON_AGG + agg::ALL_INSIDE_GATE) as u8;
        *gates.get_mut(count + 1)? = (COMMON_AGG + agg::ALL_OUTSIDE_GATE) as u8;
        metadata.add_page(c"Agg. Gate", 3, gates.get(..count + 2)?)?;
        let mut indices = [0; 12];
        let order = [
            agg::X_INSIDE_TOTAL,
            agg::X_OUTSIDE_TOTAL,
            agg::X_INSIDE_AVERAGE,
            agg::X_OUTSIDE_AVERAGE,
            agg::Y_INSIDE_TOTAL,
            agg::Y_OUTSIDE_TOTAL,
            agg::Y_INSIDE_AVERAGE,
            agg::Y_OUTSIDE_AVERAGE,
            agg::Z_INSIDE_TOTAL,
            agg::Z_OUTSIDE_TOTAL,
            agg::Z_INSIDE_AVERAGE,
            agg::Z_OUTSIDE_AVERAGE,
        ];
        for (index, local) in indices.iter_mut().zip(order) {
            *index = (COMMON_AGG + local) as u8;
        }
        metadata.add_page(c"Agg. Value", 4, &indices)?;
    }
    Some(())
}

pub(super) fn prefix(channels: ChannelCount, parameter: i32, buffer: &mut [c_char]) -> i32 {
    for ch in 0..channels.get() {
        let offset = channel_offset(ch) as i32;
        if parameter >= offset && parameter < offset + cp::COUNT as i32 {
            let Some([label, colon, terminator]) = buffer.get_mut(..3) else {
                return 0;
            };
            *label = (b'A' + ch as u8) as c_char;
            *colon = b':' as c_char;
            *terminator = 0;
            return 2;
        }
    }
    0
}

pub(super) fn needs_volts(parameter: i32) -> bool {
    parameter == gp::RANGE_MIN as i32 || parameter == gp::RANGE_MAX as i32
}

pub(super) fn append_volts(buffer: &mut [c_char], reported: i32) -> i32 {
    let end = usize::try_from(reported)
        .ok()
        .and_then(|n| n.checked_add(2).map(|end| (n, end)));
    if let Some((start, end)) = end {
        if let Some([unit, terminator]) = buffer.get_mut(start..end) {
            *unit = b'V' as c_char;
            *terminator = 0;
            return reported + 1;
        }
    }
    if let Some(first) = buffer.first_mut() {
        *first = 0;
    }
    0
}

#[cfg(test)]
mod text_tests {
    use super::*;
    #[test]
    fn prefixes_cover_channels_without_labeling_globals_or_aggregate_parameters() {
        for count in 1..=8 {
            let channels = ChannelCount::new(count).unwrap();
            for ch in 0..count {
                let mut buffer = [0; 16];
                assert_eq!(prefix(channels, channel_offset(ch) as i32, &mut buffer), 2);
                assert_eq!(
                    &buffer[..3],
                    &[(b'A' + ch as u8) as c_char, b':' as c_char, 0]
                );
            }
            for p in [
                -1,
                gp::RANGE_MIN as i32,
                super::super::COMMON_AGG as i32,
                i32::MAX,
            ] {
                assert_eq!(prefix(channels, p, &mut [0; 16]), 0);
            }
        }
    }
    #[test]
    fn short_buffers_and_invalid_lengths_do_not_panic() {
        let channels = ChannelCount::new(1).unwrap();
        assert_eq!(prefix(channels, gp::COUNT as i32, &mut [0; 2]), 0);
        assert_eq!(append_volts(&mut [], 0), 0);
        for length in [-1, 3, i32::MAX] {
            let mut buffer = [1; 4];
            assert_eq!(append_volts(&mut buffer, length), 0);
            assert_eq!(buffer[0], 0);
        }
        let mut buffer = [1; 4];
        assert_eq!(append_volts(&mut buffer, 2), 3);
        assert_eq!(&buffer[2..], &[b'V' as c_char, 0]);
    }
}
