//! Global values above the graph and a permanent, mode-aware two-row help footer.
#![forbid(unsafe_code)]
use super::*;
use distingnt_api::api::drawing::{Alignment, Shape, TextSize};

#[derive(Clone, Copy)]
pub struct UiState {
    pub selected: usize,
    pub globals: bool,
    previous: [f32; 3],
    residual: [f32; 3],
    last_written: [i16; 3],
    primed: bool,
}

impl UiState {
    pub const fn new() -> Self {
        Self {
            selected: 0,
            globals: false,
            previous: [0.5; 3],
            residual: [0.0; 3],
            last_written: [0; 3],
            primed: false,
        }
    }
}

/// Port of Common/potManager.cpp: map the remaining virtual range onto the
/// remaining physical travel in the direction of movement. Ignore jumps from
/// other screens, and snap to the stops only when moving toward them.
fn pickup(previous: f32, current: f32, value: f32) -> Option<f32> {
    if !(0.0..=1.0).contains(&previous) || !(0.0..=1.0).contains(&current) {
        return None;
    }
    let delta = current - previous;
    if delta == 0.0 || delta.abs() > 0.01 {
        return None;
    }
    let increasing = delta > 0.0;
    if increasing && current > 0.995 {
        return Some(100.0);
    }
    if !increasing && current < 0.005 {
        return Some(-100.0);
    }
    let value = value.clamp(-100.0, 100.0);
    let remaining_value = if increasing {
        100.0 - value
    } else {
        value + 100.0
    };
    let remaining_travel = if increasing { 1.0 - previous } else { previous };
    if remaining_travel <= 0.0 {
        return None;
    }
    Some((value + remaining_value / remaining_travel * delta).clamp(-100.0, 100.0))
}

pub fn handle(context: &mut Callback<'_, Ui>, data: &NtUiData) {
    let changed_mode = data.controls as u32 & ENCODER_BUTTON_R != 0
        && data.last_buttons as u32 & ENCODER_BUTTON_R == 0;
    let changed_selection = data.encoders[0] != 0;
    {
        let state = context.state_mut();
        state.ui.selected = (state.ui.selected as i32 + data.encoders[0] as i32)
            .max(0)
            .min(state.count as i32 - 1) as usize;
        if changed_mode {
            state.ui.globals = !state.ui.globals;
        }
        if changed_selection || changed_mode {
            state.ui.residual = [0.0; 3];
        }
    }
    for (pot, (mask, button, field)) in [
        (POT_L, POT_BUTTON_L, 0),
        (POT_C, POT_BUTTON_C, 2),
        (POT_R, POT_BUTTON_R, 1),
    ]
    .into_iter()
    .enumerate()
    {
        let selected = context.state().ui.selected;
        let globals = context.state().ui.globals;
        let (parameter, min, max) = if globals {
            match pot {
                0 => (Some(randomize_parameter(context.state().count)), 0, 1),
                1 => (Some(LEVEL), 0, 100),
                _ => (Some(SMOOTHING), 0, 100),
            }
        } else {
            (
                editor_parameter(context.state().count, selected, field),
                -100,
                100,
            )
        };
        let locked = parameter.is_none();
        let value = parameter
            .and_then(|p| context.read_parameters().value(p))
            .unwrap_or(0);
        let updated = {
            let ui = &mut context.state_mut().ui;
            let enabled = ui.primed
                && !changed_selection
                && !changed_mode
                && !locked
                && data.controls as u32 & mask != 0
                && (data.controls | data.last_buttons) as u32 & button == 0;
            if value != ui.last_written[pot] {
                ui.residual[pot] = 0.0;
            }
            let updated = if enabled {
                // Keep pickup and fractional movement in the parameter's own
                // units while mapping its range onto the bipolar pickup helper.
                let span = (max - min) as f32;
                let normalized =
                    (value as f32 + ui.residual[pot] - min as f32) * 200.0 / span - 100.0;
                pickup(ui.previous[pot], data.pots[pot], normalized).map(|updated| {
                    ((updated + 100.0) * span / 200.0 + min as f32)
                        .max(min as f32)
                        .min(max as f32)
                })
            } else {
                None
            };
            ui.previous[pot] = data.pots[pot];
            if let Some(updated) = updated {
                // Binary triggers switch halfway through the remaining travel
                // rather than requiring the pot to reach its physical stop.
                let whole = if max == 1 {
                    (updated + 0.5) as i16
                } else {
                    updated as i16
                };
                ui.residual[pot] = updated - whole as f32;
                ui.last_written[pot] = whole;
                Some(whole)
            } else {
                None
            }
        };
        if let (Some(updated), Some(parameter)) = (updated, parameter) {
            if updated != value {
                let _ = context.parameters().set(parameter, updated);
            }
        }
    }
    context.state_mut().ui.primed = true;
}

#[cfg(test)]
mod tests {
    use super::pickup;

    #[test]
    fn pickup_maps_remaining_travel_and_does_not_stick_at_stops() {
        assert!((pickup(0.8, 0.805, -50.0).unwrap() + 46.25).abs() < 0.001);
        assert!((pickup(0.2, 0.195, 50.0).unwrap() - 46.25).abs() < 0.001);
        assert_eq!(pickup(0.99, 0.996, -80.0), Some(100.0));
        assert_eq!(pickup(0.01, 0.004, 80.0), Some(-100.0));
        assert!(pickup(1.0, 0.995, 100.0).unwrap() < 100.0);
        assert!(pickup(0.0, 0.005, -100.0).unwrap() > -100.0);
        assert_eq!(pickup(0.5, 0.5, 25.0), None);
        assert_eq!(pickup(0.5, 0.6, 25.0), None);
        assert_eq!(pickup(f32::NAN, 0.5, 25.0), None);
        assert_eq!(pickup(0.5, f32::NAN, 25.0), None);
    }
}

fn text(canvas: &mut Canvas<'_>, x: i32, y: i32, value: &CStr) {
    canvas.text(x, y, value, 15, Alignment::Left, TextSize::Tiny);
}

fn number(canvas: &mut Canvas<'_>, x: i32, prefix: &[u8], value: i16, color: i32) {
    let mut buffer = distingnt_api::api::text::TextBuffer::<64>::new();
    buffer.format_float(prefix, value as f32, 0);
    canvas.text(
        x,
        10,
        buffer.as_c_str(),
        color,
        Alignment::Left,
        TextSize::Normal,
    );
}

pub fn draw(context: &mut Callback<'_, Ui>, mut canvas: Canvas<'_>) {
    let state = context.state();
    let memory = context.voices();
    let selected = state.ui.selected;
    // Expose the validated nonzero divisor to the optimizer so it does not
    // retain a precompiled non-PIC divide-by-zero panic helper.
    let n = state.count.max(2);
    let parameters = context.read_parameters();
    let source = read_wave(n, |index| parameters.value(index));
    canvas.shape(Shape::Rectangle, 0, 0, 255, 63, 0);
    if state.ui.globals {
        canvas.shape(Shape::Rectangle, 0, 0, 255, 12, 2);
    }
    let header_color = if state.ui.globals { 15 } else { 4 };
    number(
        &mut canvas,
        2,
        b"Randomize ",
        parameters.value(randomize_parameter(n)).unwrap_or(0),
        header_color,
    );
    number(
        &mut canvas,
        106,
        b"Level ",
        parameters.value(LEVEL).unwrap_or(50),
        header_color,
    );
    number(
        &mut canvas,
        198,
        b"Smooth ",
        parameters.value(SMOOTHING).unwrap_or(0),
        header_color,
    );
    let left = (selected * 255 / n) as i32;
    let right = ((selected + 1) * 255 / n) as i32;
    if !state.ui.globals {
        canvas.shape(Shape::Rectangle, left, 14, right, 42, 1);
    }
    // Keep the zero axis visible in both modes; segment dividers only in edit mode.
    canvas.shape(Shape::Line, 0, 28, 255, 28, 1);
    if !state.ui.globals {
        for i in 1..n {
            let x = (i * 255 / n) as i32;
            canvas.shape(Shape::Line, x, 14, x, 42, 1);
        }
    }
    // The float-coordinate API antialiases the display line. Retain fractional
    // heights instead of quantizing the curve before handing it to the host.
    // Complete the dim audio pass first so the bright editor trace is always
    // in front, even when traces intersect across neighboring segments.
    for audio in [true, false] {
        for i in 0..n {
            let x0 = (i * 255 / n) as i32;
            let x1 = ((i + 1) * 255 / n) as i32;
            let y = |t| {
                let value = if audio {
                    state.engine.audio_segment_sample(memory, i, t)
                } else {
                    state.engine.display_segment_sample(memory, i, t)
                };
                (28.0 - value * 13.0).clamp(14.0, 42.0)
            };
            let mut previous = y(0.0);
            for x in x0 + 1..=x1 {
                let current = y((x - x0) as f32 / (x1 - x0) as f32);
                canvas.shape_f(
                    Shape::Line,
                    (x - 1) as f32,
                    previous,
                    x as f32,
                    current,
                    if audio { 4.0 } else { 15.0 },
                );
                previous = current;
            }
        }
    }
    // Full-brightness boundary handles are drawn last so the antialiased
    // playback line cannot dim their outlines. Their anchor values stay fixed.
    for (x, t) in [(left, 0.0), (right, 1.0)]
        .into_iter()
        .filter(|_| !state.ui.globals)
    {
        let y = (28.0 - source.segment_sample(selected, t) * 13.0) as i32;
        canvas.shape(
            Shape::Box,
            (x - 1).max(0),
            y - 1,
            (x + 1).min(255),
            y + 1,
            15,
        );
    }
    // Match the comparator's permanent footer reservation and separator.
    canvas.shape(Shape::Rectangle, 0, 50, 255, 63, 0);
    canvas.shape(Shape::Line, 0, 50, 255, 50, 15);
    if state.ui.globals {
        text(&mut canvas, 2, 57, c"L: Randomize");
        text(&mut canvas, 91, 57, c"C: Level");
        text(&mut canvas, 193, 57, c"R: Smooth");
    } else {
        text(&mut canvas, 2, 57, c"L: Start");
        text(&mut canvas, 91, 57, c"C: Curve");
        text(&mut canvas, 193, 57, c"R: End");
    }
    text(&mut canvas, 2, 64, c"Enc L: Segment");
    text(
        &mut canvas,
        139,
        64,
        if state.ui.globals {
            c"R click: Segment"
        } else {
            c"R click: Globals"
        },
    );
}
