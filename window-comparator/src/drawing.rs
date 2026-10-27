//! Display geometry and concrete drawing through the safe API Canvas.
#![forbid(unsafe_code)]
use super::controls::EditMode;
use core::ffi::CStr;
use distingnt_api::{
    api::{
        drawing::{Alignment, Canvas, Shape, TextSize},
        text,
    },
    PARAMETER_STRING_SIZE,
};

pub(super) struct Painter<'draw> {
    canvas: Canvas<'draw>,
}
impl<'draw> Painter<'draw> {
    pub fn new(canvas: Canvas<'draw>) -> Self {
        Self { canvas }
    }
}
impl Painter<'_> {
    fn shape(&mut self, kind: Shape, x0: i32, y0: i32, x1: i32, y1: i32, color: i32) {
        self.canvas.shape(kind, x0, y0, x1, y1, color);
    }
    // Share host text-call setup rather than duplicating it across draw sites.
    #[inline(never)]
    fn text(&mut self, x: i32, y: i32, text: &CStr, color: i32, tiny: bool) {
        self.canvas.text(
            x,
            y,
            text,
            color,
            Alignment::Left,
            if tiny {
                TextSize::Tiny
            } else {
                TextSize::Normal
            },
        );
    }
    fn statistic(&mut self, label: u8, x: i32, value: f32) {
        let text = StatisticText::format(label, value);
        self.text(x, 62, text.as_c_str(), 15, false);
    }
}
pub(super) struct StatisticText {
    text: text::TextBuffer<PARAMETER_STRING_SIZE>,
}
impl StatisticText {
    pub fn format(label: u8, value: f32) -> Self {
        let mut text = Self {
            text: text::TextBuffer::new(),
        };
        text.text.format_float(&[label, b':', b' '], value, 3);
        text
    }
    pub fn as_c_str(&self) -> &CStr {
        self.text.as_c_str()
    }
}

pub(super) struct Display {
    pub selected: usize,
    pub first_line_y: i32,
    pub range_min: i8,
    pub range: u8,
    pub editable: bool,
    pub edit_mode: EditMode,
}
pub(super) struct ChannelDisplay {
    pub index: usize,
    pub left: f32,
    pub right: f32,
    pub value: f32,
}

pub(super) fn visible_channels(channels: usize, selected: usize) -> core::ops::Range<usize> {
    let visible = channels.min(4);
    let top = selected.saturating_sub(2).min(channels - visible);
    top..top + visible
}

pub(super) fn draw_channel(
    painter: &mut Painter<'_>,
    display: &Display,
    channel: ChannelDisplay,
    top: usize,
) {
    let selected = channel.index == display.selected;
    let mut bar_color = if selected { 6 } else { 2 };
    let mut color = if selected { 15 } else { 4 };
    let scale = 240.0 / display.range as f32;
    if !display.editable {
        bar_color /= 2;
        color /= 2;
    }
    let y = display.first_line_y + (channel.index - top) as i32 * 12;
    if selected {
        painter.shape(
            Shape::Rectangle,
            0,
            y + 4,
            2,
            y + 6,
            (color as f64 * 0.4) as i32,
        );
        painter.shape(Shape::Line, 1, y + 4, 1, y + 6, color);
        painter.shape(Shape::Line, 0, y + 5, 2, y + 5, color);
    }
    let label = match channel.index {
        0 => c"A",
        1 => c"B",
        2 => c"C",
        3 => c"D",
        4 => c"E",
        5 => c"F",
        6 => c"G",
        7 => c"H",
        _ => c"?",
    };
    painter.text(6, y + 9, label, color, false);
    painter.shape(Shape::Box, 15, y, 255, y + 10, bar_color);
    for i in 1..display.range as i32 {
        let x = 15 + (i as f32 * scale) as i32;
        let len = if display.range_min as i32 + i == 0 {
            2
        } else {
            1
        };
        painter.shape(Shape::Line, x, y + 1, x, y + len, 1);
        painter.shape(Shape::Line, x, y + 10 - len, x, y + 9, 1);
    }
    let inside = channel.value >= channel.left && channel.value <= channel.right;
    let lp = ((channel.left - display.range_min as f32) * scale) as i32;
    let rp = ((channel.right - display.range_min as f32) * scale) as i32;
    painter.shape(Shape::Box, 15 + lp, y, 15 + rp, y + 10, color);
    painter.shape(
        Shape::Rectangle,
        16 + lp,
        y + 1,
        14 + rp,
        y + 9,
        if inside { color / 2 } else { 0 },
    );
    if selected && 14 + rp >= 16 + lp {
        painter.shape(
            if inside { Shape::Rectangle } else { Shape::Box },
            16 + lp,
            y + 1,
            14 + rp,
            y + 9,
            color / 2,
        );
    }
    if !display.edit_mode.is_bounds() && 11 + rp >= 19 + lp {
        painter.shape(Shape::Rectangle, 19 + lp, y + 4, 11 + rp, y + 6, color);
    }
    let x = 15 + ((channel.value - display.range_min as f32) * scale) as i32;
    let palette = if inside {
        [6, 5, 4, 3, 2]
    } else {
        [2, 4, 7, 10, 15]
    };
    for (i, color) in palette.into_iter().enumerate() {
        let i = i as i32;
        painter.shape(Shape::Line, x, y + i + 1, x, y + 9 - i, color);
    }
}

pub(super) fn draw_help(
    painter: &mut Painter<'_>,
    display: &Display,
    statistics: Option<[f32; 4]>,
) {
    painter.shape(Shape::Rectangle, 0, 50, 255, 63, 0);
    if let Some(values) = statistics {
        for ((label, x), value) in [(b'L', 10), (b'R', 70), (b'C', 140), (b'W', 200)]
            .into_iter()
            .zip(values)
        {
            painter.statistic(label, x, value);
        }
    } else {
        let bounds = display.edit_mode.is_bounds();
        painter.text(2, 58, if bounds { c"Left" } else { c"Center" }, 15, true);
        painter.text(33, 64, c"Select Comparator", 15, true);
        if display.editable {
            painter.text(170, 58, c"Q: Lock", 15, true);
        } else {
            painter.text(166, 58, c"Q: Unlock", 15, true);
        }
        if bounds {
            painter.text(140, 64, c"L: Center/Width Mode", 15, true);
        } else {
            painter.text(152, 64, c"L: Bounds Mode", 15, true);
        }
        painter.text(233, 58, if bounds { c"Right" } else { c"Width" }, 15, true);
    }
    painter.shape(Shape::Line, 0, 50, 255, 50, 15);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scrolling_keeps_selected_channel_visible() {
        for channels in 1..=8 {
            for selected in 0..channels {
                let visible = visible_channels(channels, selected);
                assert!(visible.contains(&selected));
                assert!(visible.end <= channels);
                assert_eq!(visible.len(), channels.min(4));
            }
        }
        assert_eq!(visible_channels(0, 0), 0..0);
    }
}
