//! Safe plugin definition. Registration, allocation and ABI callbacks live in the API.
#![forbid(unsafe_code)]
use super::*;
use core::ffi::c_char;
use distingnt_api::api::{
    drawing::Canvas,
    json::{JsonReader, JsonWriter},
    parameters::{Audio, ReadOnly, Ui},
    plugin::{Configuration, Metadata, Plugin, Specification},
};

pub(super) struct WindowComparator;

impl Plugin for WindowComparator {
    type State = State;
    type Voice = Channel;
    type StateMemory = distingnt_api::api::state_memory::Sram;
    const GUID: u32 = multichar(b'A', b'T', b'w', b'R');
    const NAME: &'static core::ffi::CStr = c"Rust Window Comparator";
    const DESCRIPTION: &'static core::ffi::CStr = c"A window comparator";
    const TAGS: u32 = TAG_UTILITY;
    const SPECIFICATIONS: &'static [Specification] = &[Specification {
        name: c"Channels",
        min: 1,
        max: 8,
        default: 1,
        kind: TYPE_GENERIC,
    }];
    const CUSTOM_UI: u32 = POT_L | POT_R | ENCODER_L | ENCODER_R | ENCODER_BUTTON_R | POT_BUTTON_R;
    const PRESETS: bool = true;
    const INITIALIZATION_IN_DRAM: bool = true;
    const SERIALIZATION_IN_DRAM: bool = true;
    const PARAMETER_TEXT: bool = true;

    fn configuration(specs: &[i32]) -> Option<Configuration> {
        let count = ChannelCount::new(*specs.first()? as usize)?.get();
        Some(Configuration {
            parameters: count_parameters(count),
            pages: count + 3,
            page_indices: count_parameters(count),
            voices: count,
        })
    }
    fn create(specs: &[i32], rate: u32) -> Option<State> {
        let channels = ChannelCount::new(*specs.first()? as usize)?;
        Some(State {
            num_channels: channels,
            first_step: true,
            range_min: -5,
            range_max: 5,
            range: 10,
            clock: model::AudioClock::new(rate)?,
            ui: ui::UiState::new(channels),
        })
    }
    fn create_voice(_: &[i32], index: usize) -> Option<Channel> {
        Channel::new(index)
    }
    fn metadata(specs: &[i32], metadata: &mut Metadata<'_>) -> Option<()> {
        parameters::build(ChannelCount::new(*specs.first()? as usize)?.get(), metadata)
    }
    fn parameter_changed(context: &mut Callback<'_, Audio>, parameter: i32) {
        let _ = parameter_changes::changed(context, parameter as usize);
    }
    fn step(context: &mut Callback<'_, Audio>, samples: &mut [f32], frames: usize) {
        let _ = engine::step(context, samples, frames);
    }
    fn draw(context: &mut Callback<'_, Ui>, canvas: Canvas<'_>) -> bool {
        ui::draw(context, &mut drawing::Painter::new(canvas)).is_some()
    }
    fn custom_ui(context: &mut Callback<'_, Ui>, event: &NtUiData) {
        let _ = ui::handle(context, event);
    }
    fn serialise(context: &mut Callback<'_, ReadOnly>, writer: &mut JsonWriter<'_>) {
        context.state().ui.preset().save(writer);
    }
    fn deserialise(context: &mut Callback<'_, ReadOnly>, reader: &mut JsonReader<'_>) -> bool {
        let mut state = context.state().ui.preset();
        let success = state.load(reader);
        context.state_mut().ui.restore(state);
        success
    }
    fn prefix(context: &mut Callback<'_, ReadOnly>, p: i32, buffer: &mut [c_char]) -> i32 {
        parameters::prefix(context.state().num_channels, p, buffer)
    }
    fn parameter_string(
        _: &mut Callback<'_, ReadOnly>,
        p: i32,
        value: i32,
        buffer: &mut [c_char],
    ) -> i32 {
        buffer.fill(0);
        if !parameters::needs_volts(p) {
            return 0;
        }
        let Some(length) = distingnt_api::api::text::format_integer_chars(buffer, value) else {
            return 0;
        };
        parameters::append_volts(buffer, length as i32)
    }
}
