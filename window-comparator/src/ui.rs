//! UI state and orchestration. Host access uses the safe API context directly.
#![forbid(unsafe_code)]
use super::controls::{Controls, EditMode, Pot, PotMovement};
use super::{cp, drawing, model::ChannelCount, preset, Callback, NtUiData};
use distingnt_api::api::parameters::Ui;

const HELP_DRAW_COUNT: u8 = 90;

#[derive(Clone, Copy)]
pub(super) struct UiState {
    selected: usize,
    first_line_y: i32,
    edit_mode: EditMode,
    editable: bool,
    controls: Controls,
    help_draws_remaining: u8,
}
impl UiState {
    pub const fn new(channels: ChannelCount) -> Self {
        let n = channels.get();
        Self {
            selected: 0,
            first_line_y: if n < 4 { 2 + (4 - n) as i32 * 6 } else { 0 },
            edit_mode: EditMode::Bounds,
            editable: true,
            controls: Controls::new(),
            help_draws_remaining: 0,
        }
    }
    pub fn select(&mut self, delta: i8, channels: ChannelCount) {
        let channels = channels.get();
        if delta != 0 {
            self.selected = (self.selected as i32 + delta as i32)
                .max(0)
                .min(channels as i32 - 1) as usize;
            self.show_help();
        }
    }
    pub fn preset(&self) -> preset::State {
        preset::State {
            edit_mode: self.edit_mode,
            editable: self.editable,
        }
    }
    pub fn restore(&mut self, state: preset::State) {
        self.edit_mode = state.edit_mode;
        self.editable = state.editable;
    }
    pub fn selection(&self) -> (usize, EditMode, bool) {
        (self.selected, self.edit_mode, self.editable)
    }
    pub fn movement(&self, pot: Pot, data: &NtUiData, now: u32) -> Option<PotMovement> {
        self.controls.movement(pot, data, now)
    }
    pub fn finish_input(&mut self, data: &NtUiData, now: u32) {
        if self.controls.update_buttons(data, now) {
            self.editable = !self.editable;
        }
        self.controls.record_pots(data.pots);
    }
    pub fn poll(&mut self, now: u32) {
        if self.controls.poll_long_presses(now) {
            self.edit_mode.toggle();
        }
    }
    pub fn show_help(&mut self) {
        self.help_draws_remaining = HELP_DRAW_COUNT;
    }
    pub fn help_visible(&self) -> bool {
        self.help_draws_remaining > 0
    }
    pub fn help_drawn(&mut self) {
        self.help_draws_remaining = self.help_draws_remaining.saturating_sub(1);
    }
    pub fn display(&self, range_min: i8, range: u8) -> drawing::Display {
        drawing::Display {
            selected: self.selected,
            first_line_y: self.first_line_y,
            range_min,
            range,
            editable: self.editable,
            edit_mode: self.edit_mode,
        }
    }
}

fn pot_turn(context: &mut Callback<'_, Ui>, pot: Pot, movement: PotMovement) -> Option<()> {
    let (selected, mode, editable) = context.state().ui.selection();
    if !editable {
        return Some(());
    }
    let state = context.state();
    let range_min = state.range_min as f32;
    let range_max = state.range_max as f32;
    let (local, min, max) = match (pot, mode) {
        (Pot::Left, EditMode::Bounds) => (cp::LEFT, range_min, range_max),
        (Pot::Right, EditMode::Bounds) => (cp::RIGHT, range_min, range_max),
        (Pot::Left, EditMode::CenterWidth) => (cp::CENTER, range_min, range_max),
        (Pot::Right, EditMode::CenterWidth) => (cp::WIDTH, 0.0, state.range as f32),
    };
    let p = context.voices().get(selected)?.offset() + local;
    let value = movement.apply(context.read_parameters().scaled(p)?, min, max);
    context.parameters().set(p, (value * 1000.0) as i16)?;
    context.state_mut().ui.show_help();
    Some(())
}

pub(super) fn handle(context: &mut Callback<'_, Ui>, data: &NtUiData) -> Option<()> {
    let now = context.state().clock.now();
    let channels = context.state().num_channels;
    context.state_mut().ui.select(data.encoders[0], channels);
    for pot in [Pot::Left, Pot::Right] {
        if let Some(movement) = context.state().ui.movement(pot, data, now) {
            pot_turn(context, pot, movement)?;
        }
    }
    context.state_mut().ui.finish_input(data, now);
    Some(())
}

pub(super) fn draw(
    context: &mut Callback<'_, Ui>,
    painter: &mut drawing::Painter<'_>,
) -> Option<()> {
    let now = context.state().clock.now();
    context.state_mut().ui.poll(now);
    let display = {
        let state = context.state();
        state.ui.display(state.range_min, state.range)
    };
    let visible = drawing::visible_channels(context.state().num_channels.get(), display.selected);
    let top = visible.start;
    for ch in visible {
        let offset = context.voices().get(ch)?.offset();
        let channel = drawing::ChannelDisplay {
            index: ch,
            left: context.read_parameters().scaled(offset + cp::LEFT)?,
            right: context.read_parameters().scaled(offset + cp::RIGHT)?,
            value: context.voices().get(ch)?.current_value(),
        };
        drawing::draw_channel(painter, &display, channel, top);
    }
    let statistics = if context.state().ui.help_visible() {
        let offset = context.voices().get(display.selected)?.offset();
        Some([
            context.read_parameters().scaled(offset + cp::LEFT)?,
            context.read_parameters().scaled(offset + cp::RIGHT)?,
            context.read_parameters().scaled(offset + cp::CENTER)?,
            context.read_parameters().scaled(offset + cp::WIDTH)?,
        ])
    } else {
        None
    };
    drawing::draw_help(painter, &display, statistics);
    if statistics.is_some() {
        context.state_mut().ui.help_drawn();
    }
    Some(())
}
