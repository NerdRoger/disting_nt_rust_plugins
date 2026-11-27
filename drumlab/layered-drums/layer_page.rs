use core::ffi::{c_char, CStr};

use distingnt_api::{
    api::{
        metadata::{Definition, EnumStrings, Name},
        plugin::Metadata,
    },
    SCALING_NONE, UNIT_CONFIRM, UNIT_ENUM, UNIT_PERCENT,
};

pub(crate) const LAYER_RANDOMIZE: usize = 1;
pub(crate) const LAYER_LEVEL: usize = 2;
pub(crate) const LAYER_SOLO: usize = 3;
pub(crate) const LAYER_MUTE: usize = 4;
pub(crate) const LAYER_FIRST_CONTROL: usize = 5;
pub(crate) const LAYER_LEVEL_RANGE: [i16; 2] = [0, 100];

pub(crate) type PageNameBytes = [u8; 15];

static OFF_ON: EnumStrings<2> = EnumStrings::new([c"Off", c"On"]);

pub(crate) const fn page_name_storage<const LAYERS: usize, const MODELS: usize>(
    model_names: &[&CStr; MODELS],
) -> [[PageNameBytes; MODELS]; LAYERS] {
    let mut names = [[[0; 15]; MODELS]; LAYERS];
    let mut layer = 0;
    while layer < LAYERS {
        let mut model = 0;
        while model < MODELS {
            let title = &mut names[layer][model];
            title[0] = b'1' + layer as u8;
            title[1] = b'-';
            let name = model_names[model].to_bytes_with_nul();
            let mut index = 0;
            while index < name.len() {
                title[2 + index] = name[index];
                index += 1;
            }
            model += 1;
        }
        layer += 1;
    }
    names
}

pub(crate) const fn page_names<const LAYERS: usize, const MODELS: usize>(
    storage: &'static [[PageNameBytes; MODELS]; LAYERS],
) -> [[Name; MODELS]; LAYERS] {
    let mut names = [[Name::new(c""); MODELS]; LAYERS];
    let mut layer = 0;
    while layer < LAYERS {
        let mut model = 0;
        while model < MODELS {
            let Ok(name) = CStr::from_bytes_until_nul(&storage[layer][model]) else {
                panic!("layer page name must be NUL-terminated");
            };
            names[layer][model] = Name::new(name);
            model += 1;
        }
        layer += 1;
    }
    names
}

pub(crate) const fn definition(
    name: &'static CStr,
    min: i16,
    max: i16,
    initial: i16,
    unit: u8,
    scale: u8,
) -> Definition {
    Definition::new(name, min, max, initial, unit, scale, None).unwrap()
}

pub(crate) fn add_layer_controls(metadata: &mut Metadata<'_>, base: usize) -> Option<()> {
    let randomize = definition(c"Randomize", 0, 1, 0, UNIT_CONFIRM, SCALING_NONE);
    metadata.set_parameter(base + LAYER_RANDOMIZE, randomize)?;
    let [min, max] = LAYER_LEVEL_RANGE;
    let level = definition(c"Level", min, max, max, UNIT_PERCENT, SCALING_NONE);
    metadata.set_parameter(base + LAYER_LEVEL, level)?;
    for (offset, name) in [(LAYER_SOLO, c"Solo"), (LAYER_MUTE, c"Mute")] {
        let toggle = Definition::new(name, 0, 1, 0, UNIT_ENUM, SCALING_NONE, Some(OFF_ON.table()))?;
        metadata.set_parameter(base + offset, toggle)?;
    }
    Some(())
}

pub(crate) fn write_text(output: &mut [c_char], text: &[u8]) -> i32 {
    let length = text.len();
    let Some(output) = output.get_mut(..=length) else {
        return 0;
    };
    for (destination, &byte) in output.iter_mut().zip(text) {
        *destination = byte as c_char;
    }
    output[length] = 0;
    length as i32
}

pub(crate) fn write_model_name(output: &mut [c_char], models: &[&CStr], value: i32) -> i32 {
    models
        .get(value as usize)
        .map_or(0, |name| write_text(output, name.to_bytes()))
}

pub(crate) fn write_randomize_label(output: &mut [c_char], value: i32) -> i32 {
    write_text(output, if value == 0 { b"Ready" } else { b"Randomize?" })
}
