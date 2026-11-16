//! Global values above the graph and a permanent, mode-aware encoder help footer.
#![forbid(unsafe_code)]
use super::*;
use distingnt_api::api::drawing::{Alignment, Shape, TextSize};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    Segment,
    Warp,
    Tuning,
}

impl EditMode {
    fn next(self) -> Self {
        match self {
            Self::Segment => Self::Warp,
            Self::Warp => Self::Tuning,
            Self::Tuning => Self::Segment,
        }
    }
}

#[derive(Clone, Copy)]
pub struct UiState {
    pub selected: usize,
    pub mode: EditMode,
    previous: [f32; 3],
    reset_resync: bool,
    primed: bool,
    reset_hold: [usize; 3],
}

impl UiState {
    pub const fn new() -> Self {
        Self {
            selected: 0,
            mode: EditMode::Segment,
            previous: [0.5; 3],
            reset_resync: false,
            primed: false,
            reset_hold: [0; 3],
        }
    }

    #[inline]
    fn needs_handling(&self, data: &NtUiData) -> bool {
        const ACTIVITY: u32 =
            POT_L | POT_C | POT_R | POT_BUTTON_L | POT_BUTTON_C | POT_BUTTON_R | ENCODER_BUTTON_R;
        !self.primed
            || (self.reset_resync && self.reset_hold == [0; 3])
            || (data.controls | data.last_buttons) as u32 & ACTIVITY != 0
            || data.encoders != [0, 0]
    }

    pub fn advance(&mut self, frames: usize) {
        for remaining in &mut self.reset_hold {
            *remaining = remaining.saturating_sub(frames);
        }
    }
}

/// Current bindings in left/centre/right pot order.
fn bindings(state: &State) -> [Option<usize>; 3] {
    match state.ui.mode {
        EditMode::Segment => {
            [0, 2, 1].map(|field| editor_parameter(state.count, state.ui.selected, field))
        }
        EditMode::Warp => [Some(WARP_POSITION), Some(WARP_AMOUNT), Some(SMOOTHING)],
        EditMode::Tuning => [Some(PITCH), Some(FINE), Some(LEVEL)],
    }
}

distingnt_api::nt_dram_section! {
/// Give the firmware the represented values so it owns potentiometer takeover.
pub fn setup_values(context: &mut Callback<'_, Ui>) -> NtFloat3 {
    let values = bindings(context.state()).map(|parameter| {
        let p = context.read_parameters();
        let Some(parameter) = parameter else { return 0.5; };
        let Some(d) = p.definition(parameter) else { return 0.5; };
        let span = i32::from(d.max) - i32::from(d.min);
        if span <= 0 { return 0.5; }
        let value = p.value(parameter).unwrap_or(d.default).clamp(d.min, d.max);
        (i32::from(value) - i32::from(d.min)) as f32 / span as f32
    });
    let ui = &mut context.state_mut().ui;
    ui.previous = values;
    ui.primed = true;
    values
}
}

/// The host has already applied pickup; map its position directly to the parameter.
fn pot_value(position: f32, min: i16, max: i16) -> Option<i16> {
    if !position.is_finite() || max <= min {
        return None;
    }
    let value = min as f32 + position.clamp(0.0, 1.0) * (i32::from(max) - i32::from(min)) as f32;
    Some(if value < 0.0 {
        (value - 0.5) as i16
    } else {
        (value + 0.5) as i16
    })
}

// Keep the no-activity dispatcher in ITC and edit work in DRAM.
pub fn handle(context: &mut Callback<'_, Ui>, data: &NtUiData) -> bool {
    if !context.state().ui.needs_handling(data) {
        context.state_mut().ui.previous = data.pots;
        return false;
    }
    handle_active(context, data)
}

distingnt_api::nt_dram_section! {
fn handle_active(context: &mut Callback<'_, Ui>, data: &NtUiData) -> bool {
    let changed_mode = data.controls as u32 & ENCODER_BUTTON_R != 0
        && data.last_buttons as u32 & ENCODER_BUTTON_R == 0;
    let changed_selection = data.encoders[0] != 0;
    let mut edited = changed_mode || changed_selection;
    let reset_finished = context.state().ui.reset_resync && context.state().ui.reset_hold == [0; 3];
    let mut resync = changed_mode || changed_selection || reset_finished;
    if reset_finished { context.state_mut().ui.reset_resync = false; }
    {
        let state = context.state_mut();
        let count = state.count.clamp(2, dsp::MAX_SEGMENTS) as i32;
        let selection = state.ui.selected as i32 + data.encoders[0] as i32;
        state.ui.selected = if state.ui.mode == EditMode::Segment {
            selection.rem_euclid(count) as usize
        } else {
            selection.max(0).min(count - 1) as usize
        };
        if changed_mode {
            state.ui.mode = state.ui.mode.next();
        }
    }
    let midi = context
        .read_parameters()
        .value(MIDI_CHANNEL)
        .unwrap_or(0)
        .clamp(0, 17)
        != 0;
    let parameters = bindings(context.state());
    for pot in 0..3 {
        let (mask, button) = match pot {
            0 => (POT_L, POT_BUTTON_L),
            1 => (POT_C, POT_BUTTON_C),
            _ => (POT_R, POT_BUTTON_R),
        };
        let parameter = parameters[pot];
        let mode = context.state().ui.mode;
        // Use the same definition for setup, editing, and reset bounds.
        let range = parameter.and_then(|p| {
            context
                .read_parameters()
                .definition(p)
                .map(|d| (d.min, d.max, d.default))
        });
        let (min, max, default) = range.unwrap_or((-100, 100, 0));
        let locked = range.is_none() || (parameter == Some(PITCH) && midi);
        let value = parameter
            .and_then(|p| context.read_parameters().value(p))
            .unwrap_or(0);
        let pressed = data.controls as u32 & button != 0 && data.last_buttons as u32 & button == 0;
        // Suppress release-induced movement for 100 ms after the button is let
        // go too, even if it was held longer than the initial press interval.
        if !locked && (data.controls | data.last_buttons) as u32 & button != 0 {
            let hold_frames = (context.state().sample_rate as usize).div_ceil(10);
            context.state_mut().ui.reset_hold[pot] = hold_frames;
            context.state_mut().ui.reset_resync = true;
        }
        if pressed && !locked {
            if let Some(parameter) = parameter {
                let reset_value = if mode == EditMode::Segment && pot != 1 {
                    0
                } else {
                    default
                };
                // Reset through the host, then synchronize firmware takeover.
                // Holding/releasing cannot undo the reset.
                {
                    let ui = &mut context.state_mut().ui;
                    ui.previous[pot] = data.pots[pot];
                    resync = true;
                }
                if value != reset_value {
                    let _ = context.parameters().set(parameter, reset_value);
                    edited = true;
                }
                continue;
            }
        }
        let updated = {
            let ui = &mut context.state_mut().ui;
            let enabled = ui.primed
                && !changed_selection
                && !changed_mode
                && !reset_finished
                && !locked
                && ui.reset_hold[pot] == 0
                && data.controls as u32 & mask != 0
                && (data.controls | data.last_buttons) as u32 & button == 0
                && data.pots[pot] != ui.previous[pot];
            ui.previous[pot] = data.pots[pot];
            if enabled { pot_value(data.pots[pot], min, max) } else { None }
        };
        if let (Some(updated), Some(parameter)) = (updated, parameter) {
            if updated != value {
                let _ = context.parameters().set(parameter, updated);
                edited = true;
                // Keep the host's represented values synchronized with accepted edits.
                resync = true;
            }
        }
    }
    context.state_mut().ui.primed = true;
    if resync {
        context.request_setup_ui();
    }
    edited
}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_gate_handles_initialization_changes_held_buttons_and_releases() {
        let mut ui = UiState::new();
        let mut data = NtUiData {
            pots: [0.5; 3],
            controls: 0,
            last_buttons: 0,
            encoders: [0; 2],
            unused: [0; 2],
        };
        assert!(ui.needs_handling(&data));
        ui.primed = true;
        assert!(!ui.needs_handling(&data));
        // Raw noise alone must not wake the editor: use the host movement flags.
        data.pots = [0.50001, 0.49999, 0.5];
        assert!(!ui.needs_handling(&data));
        for flag in [
            POT_L,
            POT_C,
            POT_R,
            POT_BUTTON_L,
            POT_BUTTON_C,
            POT_BUTTON_R,
            ENCODER_BUTTON_R,
        ] {
            data.controls = flag as u16;
            assert!(ui.needs_handling(&data));
            data.last_buttons = flag as u16;
            assert!(ui.needs_handling(&data));
            data.controls = 0;
            assert!(ui.needs_handling(&data));
            data.last_buttons = 0;
        }
        for index in 0..2 {
            for delta in [-1, 1] {
                data.encoders[index] = delta;
                assert!(ui.needs_handling(&data));
                data.encoders[index] = 0;
            }
        }
        assert!(!ui.needs_handling(&data));
    }

    #[test]
    fn host_positions_map_directly_to_parameter_ranges() {
        assert_eq!(pot_value(0.25, -100, 100), Some(-50));
        assert_eq!(pot_value(0.75, -100, 100), Some(50));
        assert_eq!(pot_value(0.0, -100, 100), Some(-100));
        assert_eq!(pot_value(1.0, -100, 100), Some(100));
        assert_eq!(pot_value(0.5, 0, 255), Some(128));
        assert_eq!(pot_value(f32::NAN, -100, 100), None);
        assert_eq!(pot_value(0.5, 10, 10), None);
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

fn level_db(canvas: &mut Canvas<'_>, x: i32, value: i16, color: i32) {
    let mut buffer = [0u8; 32];
    let text = if value <= levels::LEVEL_MIN {
        c"Level -inf"
    } else {
        buffer[..6].copy_from_slice(b"Level ");
        let Some(length) = distingnt_api::api::text::format_integer(&mut buffer[6..], value as i32)
        else {
            return;
        };
        let Some(suffix) = buffer.get_mut(6 + length..6 + length + 4) else {
            return;
        };
        suffix.copy_from_slice(b" dB\0");
        let Ok(text) = CStr::from_bytes_until_nul(&buffer) else {
            return;
        };
        text
    };
    canvas.text(x, 10, text, color, Alignment::Left, TextSize::Normal);
}

fn pitch_note(canvas: &mut Canvas<'_>, x: i32, value: i16, color: i32) {
    // Match the NT's MIDI octave convention: 36 = C2, 37 = C#2.
    let note = value.clamp(0, 127) as usize;
    let names: [&[u8]; 12] = [
        b"Pitch C",
        b"Pitch C#",
        b"Pitch D",
        b"Pitch D#",
        b"Pitch E",
        b"Pitch F",
        b"Pitch F#",
        b"Pitch G",
        b"Pitch G#",
        b"Pitch A",
        b"Pitch A#",
        b"Pitch B",
    ];
    number(canvas, x, names[note % 12], (note / 12) as i16 - 1, color);
}

pub fn draw(context: &mut Callback<'_, Ui>, mut canvas: Canvas<'_>) {
    let state = context.state();
    let memory = &state.tables[..];
    let selected = state.ui.selected;
    // Expose the validated nonzero divisor to the optimizer so it does not
    // retain a precompiled non-PIC divide-by-zero panic helper.
    let n = state.count.max(2);
    let parameters = context.read_parameters();
    let source = read_wave(n, |index| parameters.value(index));
    canvas.shape(Shape::Rectangle, 0, 0, 255, 63, 0);
    canvas.shape(Shape::Rectangle, 0, 0, 255, 12, 2);
    let header_color = 15;
    let header: [(i32, &[u8], usize, i16); 3] = match state.ui.mode {
        EditMode::Segment => [
            (
                2,
                b"Left ",
                editor_parameter(n, selected, 0).unwrap_or(WAVEFORM_START),
                0,
            ),
            (102, b"Curve ", curve_parameter(selected), 0),
            (
                190,
                b"Right ",
                editor_parameter(n, selected, 1).unwrap_or(WAVEFORM_START),
                0,
            ),
        ],
        EditMode::Tuning => [
            (2, b"Pitch ", PITCH, 57),
            (102, b"Fine ", FINE, 0),
            (180, b"Level ", LEVEL, levels::LEVEL_DEFAULT),
        ],
        EditMode::Warp => [
            (2, b"Warp Pos ", WARP_POSITION, dsp::WARP_POSITION_DEFAULT),
            (102, b"Warp Amt ", WARP_AMOUNT, 0),
            (198, b"Smooth ", SMOOTHING, 0),
        ],
    };
    for (x, prefix, parameter, default) in header {
        let value = parameters.value(parameter).unwrap_or(default);
        if parameter == PITCH {
            let midi = parameters.value(MIDI_CHANNEL).unwrap_or(0).clamp(0, 17) != 0;
            pitch_note(&mut canvas, x, value, if midi { 5 } else { header_color });
        } else if parameter == LEVEL {
            level_db(&mut canvas, x, value, header_color);
        } else {
            number(&mut canvas, x, prefix, value, header_color);
        }
    }
    let left = (source.boundary_position(selected) * 255.0) as i32;
    let right = (source.boundary_position(selected + 1) * 255.0) as i32;
    if state.ui.mode == EditMode::Segment {
        canvas.shape(Shape::Rectangle, left, 14, right, 42, 1);
    }
    // Keep the dim grid visible in every mode, including while adjusting warp.
    canvas.shape(Shape::Line, 0, 28, 255, 28, 1);
    for i in 1..n {
        let x = (source.boundary_position(i) * 255.0) as i32;
        for y in (14..=42).step_by(3) {
            canvas.shape(Shape::Point, x, y, x, y, 1);
        }
    }
    // The float-coordinate API antialiases the display line. Retain fractional
    // heights instead of quantizing the curve before handing it to the host.
    let y = |phase: f32| {
        let value = state.engine.editor_phase_sample(memory, phase);
        (28.0 - value * 13.0).clamp(14.0, 42.0)
    };
    for segment in 0..n {
        let start = source.boundary_position(segment);
        let end = source.boundary_position(segment + 1);
        let mut previous_x = start * 255.0;
        let end_x = end * 255.0;
        let mut previous_y = y(start);
        let mut pixel = previous_x as usize + 1;
        while (pixel as f32) < end_x {
            let current = y(pixel as f32 / 255.0);
            canvas.shape_f(
                Shape::Line,
                previous_x,
                previous_y,
                pixel as f32,
                current,
                15.0,
            );
            previous_x = pixel as f32;
            previous_y = current;
            pixel += 1;
        }
        // Keep a vertex at each exact boundary, even for segments narrower
        // than one display pixel.
        canvas.shape_f(Shape::Line, previous_x, previous_y, end_x, y(end), 15.0);
    }
    if state.ui.mode == EditMode::Warp {
        // Draw over the waveform so the full-brightness core and black margins
        // remain visible. Keep the complete four-pixel marker on screen at either edge.
        let center = source.warp_position.clamp(0, dsp::WARP_POSITION_MAX) as i32;
        let marker_left = (center - 1).clamp(0, 252);
        canvas.shape(Shape::Rectangle, marker_left, 14, marker_left + 3, 42, 0);
        canvas.shape(
            Shape::Rectangle,
            marker_left + 1,
            14,
            marker_left + 2,
            42,
            15,
        );
    }
    // Full-brightness boundary handles are drawn last so the antialiased
    // waveform line cannot dim their outlines. Their anchor values stay fixed.
    for (x, t) in [(left, 0.0), (right, 1.0)]
        .into_iter()
        .filter(|_| state.ui.mode == EditMode::Segment)
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
    if state.ui.mode == EditMode::Segment {
        text(&mut canvas, 42, 64, c"Turn: Select");
    }
    text(&mut canvas, 159, 64, c"Push: Mode");
    #[cfg(feature = "profiling")]
    {
        // Temporary profiling overlay: retain the normal help rendering above so
        // removing this overlay restores it without rebuilding the help layout.
        canvas.shape(Shape::Rectangle, 0, 50, 255, 63, 0);
        let mut buffer = [0u8; 40];
        buffer[..12].copy_from_slice(b"Render cyc: ");
        if let Some(length) = distingnt_api::api::text::format_integer(
            &mut buffer[12..],
            state.profiling.render_cycles.average.min(i32::MAX as u32) as i32,
        ) {
            if let Some(bytes) = buffer.get(..12 + length + 1) {
                if let Ok(value) = CStr::from_bytes_with_nul(bytes) {
                    canvas.text(2, 62, value, 15, Alignment::Left, TextSize::Normal);
                }
            }
        }
    }
}
