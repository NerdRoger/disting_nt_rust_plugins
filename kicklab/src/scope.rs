use core::ffi::CStr;

use distingnt_api::api::{
    drawing::{Alignment, Canvas, Shape, TextSize},
    math::exp2,
    plugin::RegionInitializer,
    text::{format_float, format_integer},
};

const WIDTH: usize = 254;
const LEFT: i32 = 1;
const RIGHT: i32 = LEFT + WIDTH as i32 - 1;
const TOP: i32 = 12;
const BOTTOM: i32 = 50;
const CENTRE: i32 = (TOP + BOTTOM) / 2;
const LIMIT_COLOUR: i32 = 1;
const WAVE_COLOUR: i32 = 15;
const END_COLOUR: i32 = 8;
const FOOTER_Y: i32 = BOTTOM + 10;
const AMPLITUDE: f32 = (CENTRE - TOP) as f32;
const SILENT: i16 = i16::MIN;
const BELOW_METER: i16 = -1201;

#[derive(Clone, Copy)]
pub struct Scope {
    columns: [[i8; 2]; WIDTH],
    peak: i16,
}

impl Scope {
    pub const COLUMNS: usize = WIDTH;

    pub fn initialize(destination: &mut RegionInitializer<'_, Self>) {
        distingnt_api::initialize_region!(
            destination,
            Scope { peak: SILENT },
            arrays {
                columns: |_| [0, 0]
            }
        );
    }

    #[cfg(test)]
    pub fn capture(&mut self, samples: &mut [f32], rate: u32, mut finish: impl FnMut(&mut f32)) {
        let peak = self.capture_columns(samples, rate, 0..WIDTH, |sample, _| finish(sample));
        self.finish_capture(peak);
    }

    pub fn capture_columns(
        &mut self,
        samples: &mut [f32],
        rate: u32,
        columns: core::ops::Range<usize>,
        mut finish: impl FnMut(&mut f32, usize),
    ) -> f32 {
        let plot_frames = rate as usize;
        let mut peak = 0.0_f32;
        let first_column = columns.start;
        for (index, column) in self
            .columns
            .get_mut(columns)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let x = first_column + index;
            let start = (x * plot_frames / WIDTH).min(samples.len());
            let end = ((x + 1) * plot_frames / WIDTH).min(samples.len());
            let slice = samples.get_mut(start..end).unwrap_or_default();
            let Some((first, rest)) = slice.split_first_mut() else {
                *column = [0, 0];
                continue;
            };
            finish(first, start);
            let mut min = *first;
            let mut max = *first;
            for (index, sample) in rest.iter_mut().enumerate() {
                finish(sample, start + index + 1);
                min = min.min(*sample);
                max = max.max(*sample);
            }
            peak = peak.max(min.abs()).max(max.abs());
            *column = [displacement(max), displacement(min)];
        }
        peak
    }

    pub fn finish_capture(&mut self, peak: f32) {
        self.peak = peak_dbfs(peak);
    }

    pub fn draw(&self, mut canvas: Canvas<'_>, buffers_used: i32, buffer_waits: i32) {
        canvas.shape(Shape::Rectangle, 0, TOP, 255, 63, 0);
        canvas.shape(Shape::Line, LEFT, TOP, RIGHT, TOP, LIMIT_COLOUR);
        canvas.shape(Shape::Line, LEFT, BOTTOM, RIGHT, BOTTOM, LIMIT_COLOUR);
        // Batch identical adjacent slices, especially the flat tail.
        let mut left = LEFT;
        for run in self.columns.chunk_by(|a, b| a == b) {
            let [max, min] = run[0];
            let right = left + run.len() as i32 - 1;
            let mut top = CENTRE - i32::from(max);
            let mut bottom = CENTRE - i32::from(min);
            if bottom - top >= 2 {
                let cap = if left == right {
                    Shape::Point
                } else {
                    Shape::Line
                };
                canvas.shape(cap, left, top, right, top, END_COLOUR);
                canvas.shape(cap, left, bottom, right, bottom, END_COLOUR);
                top += 1;
                bottom -= 1;
            }
            let shape = if left != right {
                Shape::Rectangle
            } else if top == bottom {
                Shape::Point
            } else {
                Shape::Line
            };
            canvas.shape(shape, left, top, right, bottom, WAVE_COLOUR);
            left = right + 1;
        }

        let mut buffer = [0; 64];
        let peak = match self.peak {
            SILENT => c"Peak: -inf dBFS",
            BELOW_METER => c"Peak: <-120 dBFS",
            value => format_peak(&mut buffer, value).unwrap_or(c"Peak: -- dBFS"),
        };
        canvas.text(LEFT, FOOTER_Y, peak, 15, Alignment::Left, TextSize::Tiny);
        let buffers = format_buffer_usage(&mut buffer, buffers_used, buffer_waits)
            .unwrap_or(c"Buffers - Used: --  Waits: --");
        canvas.text(
            128,
            FOOTER_Y,
            buffers,
            15,
            Alignment::Centre,
            TextSize::Tiny,
        );
        canvas.text(
            RIGHT,
            FOOTER_Y,
            c"1000 ms",
            15,
            Alignment::Right,
            TextSize::Tiny,
        );
    }
}

fn format_buffer_usage(buffer: &mut [u8; 64], used: i32, waits: i32) -> Option<&CStr> {
    const PREFIX: &[u8] = b"Buffers - Used: ";
    const WAIT_LABEL: &[u8] = b"  Waits: ";
    let (prefix, rest) = buffer.split_at_mut(PREFIX.len());
    prefix.copy_from_slice(PREFIX);
    let digits = format_integer(rest, used)?;
    let (label, rest) = rest
        .get_mut(digits..)?
        .split_at_mut_checked(WAIT_LABEL.len())?;
    label.copy_from_slice(WAIT_LABEL);
    format_integer(rest, waits)?;
    CStr::from_bytes_until_nul(buffer).ok()
}

fn displacement(sample: f32) -> i8 {
    let offset = sample.clamp(-1.0, 1.0) * AMPLITUDE;
    (offset + 0.5_f32.copysign(offset)) as i8
}

fn peak_dbfs(peak: f32) -> i16 {
    if peak == 0.0 {
        return SILENT;
    }
    if peak < 0.000001 {
        return BELOW_METER;
    }
    // Compare with the half-step thresholds to round to the nearest 0.1 dB.
    let (mut low, mut high) = (-1200, 0);
    while low < high {
        let middle = low + (high - low + 1) / 2;
        let threshold = exp2((f32::from(middle) - 0.5) * (core::f32::consts::LOG2_10 / 200.0));
        if peak >= threshold {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    low
}

fn format_peak(buffer: &mut [u8; 64], peak: i16) -> Option<&CStr> {
    const PREFIX: &[u8] = b"Peak: ";
    const SUFFIX: &[u8] = b" dBFS\0";
    buffer[..PREFIX.len()].copy_from_slice(PREFIX);
    let end = PREFIX.len() + format_float(&mut buffer[PREFIX.len()..], peak as f32 * 0.1, 1)?;
    buffer
        .get_mut(end..end + SUFFIX.len())?
        .copy_from_slice(SUFFIX);
    CStr::from_bytes_until_nul(buffer).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_keeps_both_extrema_without_pulling_one_sided_slices_to_zero() {
        let mut scope = Scope {
            columns: [[0, 0]; WIDTH],
            peak: SILENT,
        };
        scope.capture(
            &mut [0.1, -1.0, 0.8, 0.1, -0.2, -0.7, -0.1, -0.4],
            (WIDTH * 4) as u32,
            |_| {},
        );
        assert_eq!(scope.columns[0], [15, -19]);
        assert_eq!(scope.columns[1], [-2, -13]);
        assert!(scope.columns[2..].iter().all(|&column| column == [0, 0]));
        assert_eq!(scope.peak, 0);
    }
}
