//! Independent framework tests; this module simulates the external C++ host.
extern crate std;
use super::super::metadata::{Definition, EnumStrings};
use super::*;
use std::{
    alloc::{alloc, dealloc},
    cell::Cell,
};

#[derive(Clone, Copy)]
enum Mode {
    Valid,
    MissingParameter,
    BadPage,
    BadState,
    BadVoice,
    TooManyParameters,
    NoParameters,
}
std::thread_local! { static MODE: Cell<Mode> = const { Cell::new(Mode::Valid) }; }
std::thread_local! {
    static STATIC_BYTES: Cell<u32> = const { Cell::new(16) };
    static STATIC_CALLS: Cell<usize> = const { Cell::new(0) };
    static STATIC_ZEROED: Cell<bool> = const { Cell::new(false) };
}

struct Example;
impl Plugin for Example {
    type State = u32;
    type Voice = u32;
    type StateMemory = Sram;
    const GUID: u32 = 1;
    const NAME: &'static CStr = c"Framework test";
    const DESCRIPTION: &'static CStr = c"";
    const TAGS: u32 = TAG_UTILITY;
    const SPECIFICATIONS: &'static [Specification] = &[];
    const INITIALIZATION_IN_DRAM: bool = true;
    const STATIC_INITIALIZATION: bool = true;
    fn calculate_static_requirements() -> StaticRequirements {
        StaticRequirements {
            dram: STATIC_BYTES.with(Cell::get),
        }
    }
    fn initialise(memory: StaticMemory<'_>) {
        STATIC_CALLS.with(|calls| calls.set(calls.get() + 1));
        STATIC_ZEROED.with(|zeroed| zeroed.set(memory.dram.iter().all(|&byte| byte == 0)));
        for (index, byte) in memory.dram.iter_mut().enumerate() {
            *byte = index as u8 ^ 0x5a;
        }
    }
    fn configuration(_: &[i32]) -> Option<Configuration> {
        if MODE.with(|mode| matches!(mode.get(), Mode::NoParameters)) {
            return Some(Configuration {
                parameters: 0,
                pages: 0,
                page_indices: 0,
                voices: 0,
            });
        }
        Some(Configuration {
            parameters: if MODE.with(|mode| matches!(mode.get(), Mode::TooManyParameters)) {
                257
            } else {
                1
            },
            pages: 1,
            page_indices: 1,
            voices: 2,
        })
    }
    fn create(_: &[i32], _: u32) -> Option<u32> {
        (!MODE.with(|mode| matches!(mode.get(), Mode::BadState))).then_some(123)
    }
    fn create_voice(_: &[i32], index: usize) -> Option<u32> {
        if index == 1 && MODE.with(|mode| matches!(mode.get(), Mode::BadVoice)) {
            return None;
        }
        Some(index as u32)
    }
    fn metadata(_: &[i32], metadata: &mut Metadata<'_>) -> Option<()> {
        if MODE.with(|mode| matches!(mode.get(), Mode::NoParameters)) {
            return Some(());
        }
        if MODE.with(|mode| matches!(mode.get(), Mode::MissingParameter)) {
            return Some(());
        }
        metadata.set_parameter(
            0,
            Definition::new(c"Value", 0, 10, 0, UNIT_NONE, SCALING_NONE, None)?,
        )?;
        metadata.add_page(
            c"Test",
            1,
            if MODE.with(|mode| matches!(mode.get(), Mode::BadPage)) {
                &[1]
            } else {
                &[0]
            },
        )
    }
}
type Bridge = Registration<Example, 0>;

std::thread_local! {
    static SYSEX_MESSAGES: std::cell::RefCell<std::vec::Vec<std::vec::Vec<u8>>> = const {
        std::cell::RefCell::new(std::vec::Vec::new())
    };
}

/// Generic probe exercises every factory endpoint without plugin-specific logic.
struct AllCallbacks<const ENABLED: bool>;
impl<const ENABLED: bool> Plugin for AllCallbacks<ENABLED> {
    type State = [u8; 8];
    type Voice = ();
    type StateMemory = Sram;
    const GUID: u32 = 2;
    const NAME: &'static CStr = c"All callbacks";
    const DESCRIPTION: &'static CStr = c"";
    const TAGS: u32 = 0;
    const SPECIFICATIONS: &'static [Specification] = &[];
    const CUSTOM_UI: u32 = if ENABLED { POT_L } else { 0 };
    const PRESETS: bool = ENABLED;
    const PARAMETER_TEXT: bool = ENABLED;
    const MIDI: bool = ENABLED;
    const MIDI_SYSEX: bool = ENABLED;
    const STRING_PARAMETERS: bool = ENABLED;
    const STATIC_INITIALIZATION: bool = ENABLED;
    fn configuration(_: &[i32]) -> Option<Configuration> {
        Some(Configuration {
            parameters: 3,
            pages: 0,
            page_indices: 0,
            voices: 0,
        })
    }
    fn create(_: &[i32], _: u32) -> Option<Self::State> {
        Some([0xa5; 8])
    }
    fn create_voice(_: &[i32], _: usize) -> Option<()> {
        Some(())
    }
    fn metadata(_: &[i32], metadata: &mut Metadata<'_>) -> Option<()> {
        for index in 0..3 {
            metadata.set_parameter(
                index,
                Definition::new(c"Text", 0, 0, 0, UNIT_STRING, SCALING_NONE, None)?,
            )?;
        }
        Some(())
    }
    fn has_custom_ui(context: &mut Context<'_, Self::State, (), ReadOnly>) -> u32 {
        if context.state()[0] == 0 {
            0
        } else {
            POT_L | POT_R
        }
    }
    fn midi_sysex(bytes: &[u8]) {
        SYSEX_MESSAGES.with(|messages| messages.borrow_mut().push(bytes.to_vec()));
    }
    fn string_parameter_buffer<'a>(
        context: &'a mut Context<'_, Self::State, (), ReadOnly>,
        p: i32,
    ) -> Option<&'a mut [u8]> {
        match p {
            0 => Some(context.state_mut()),
            1 => Some(&mut context.state_mut()[..0]),
            _ => None,
        }
    }
}

#[test]
fn all_eighteen_header_endpoints_are_available_and_optional_hooks_can_be_disabled() {
    fn availability(factory: &NtFactory) -> [bool; 18] {
        [
            factory.calculate_static_requirements.is_some(),
            factory.initialise.is_some(),
            factory.calculate_requirements.is_some(),
            factory.construct.is_some(),
            factory.parameter_changed.is_some(),
            factory.step.is_some(),
            factory.draw.is_some(),
            factory.midi_realtime.is_some(),
            factory.midi_message.is_some(),
            factory.has_custom_ui.is_some(),
            factory.custom_ui.is_some(),
            factory.setup_ui.is_some(),
            factory.serialise.is_some(),
            factory.deserialise.is_some(),
            factory.midi_sysex.is_some(),
            factory.parameter_ui_prefix.is_some(),
            factory.parameter_string.is_some(),
            factory.string_parameter_buffer.is_some(),
        ]
    }
    assert_eq!(
        availability(&Registration::<AllCallbacks<true>, 0>::FACTORY),
        [true; 18]
    );
    assert_eq!(
        availability(&Registration::<AllCallbacks<false>, 0>::FACTORY),
        [
            false, false, true, true, true, true, true, false, false, false, false, false, false,
            false, false, false, false, false
        ]
    );
}

#[test]
fn sysex_forwards_exact_factory_wide_bytes_and_handles_empty_or_null_input() {
    SYSEX_MESSAGES.with(|messages| messages.borrow_mut().clear());
    let callback = Registration::<AllCallbacks<true>, 0>::FACTORY
        .midi_sysex
        .unwrap();
    let bytes = [0xf0, 0x7d, 0, 0x7f, 0xf7];
    unsafe {
        callback(bytes.as_ptr(), bytes.len() as u32);
        callback(ptr::null(), 0);
        callback(ptr::null(), 1);
    }
    SYSEX_MESSAGES.with(|messages| assert_eq!(*messages.borrow(), [bytes.to_vec(), std::vec![]]));
}

#[test]
fn editable_strings_are_bounded_terminated_and_host_edits_reach_plugin_state() {
    let allocation = Allocation::for_plugin::<AllCallbacks<true>>();
    let base = allocation.construct_plugin::<AllCallbacks<true>>();
    assert!(!base.is_null());
    let factory = &Registration::<AllCallbacks<true>, 0>::FACTORY;
    let callback = factory.string_parameter_buffer.unwrap();
    let mut output = ptr::null_mut();
    unsafe {
        assert_eq!(callback(base, 0, &mut output), 8);
        assert!(!output.is_null());
        assert_eq!(
            slice::from_raw_parts(output.cast::<u8>(), 8),
            &[0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0]
        );
        // Simulate synchronous host edits, including a byte that is invalid UTF-8.
        *output = 0xffu8 as c_char;
        *output.add(1) = 0;
    }
    {
        let context = unsafe { Context::<[u8; 8], (), ReadOnly>::admit(base) };
        assert_eq!(&context.state()[..2], &[0xff, 0]);
    }
    for parameter in [-1, 1, 2, 3, i32::MAX] {
        unsafe {
            assert_eq!(callback(base, parameter, &mut output), 0);
        }
        assert!(output.is_null());
    }
    unsafe {
        assert_eq!(callback(base, 0, ptr::null_mut()), 0);
    }
}

#[test]
fn custom_ui_mask_can_depend_on_instance_state_but_stays_within_declared_controls() {
    let allocation = Allocation::for_plugin::<AllCallbacks<true>>();
    let base = allocation.construct_plugin::<AllCallbacks<true>>();
    let callback = Registration::<AllCallbacks<true>, 0>::FACTORY
        .has_custom_ui
        .unwrap();
    unsafe {
        assert_eq!(callback(base), POT_L);
    }
    {
        let mut context = unsafe { Context::<[u8; 8], (), ReadOnly>::admit(base) };
        context.state_mut()[0] = 0;
    }
    unsafe {
        assert_eq!(callback(base), 0);
    }
}

#[test]
fn static_factory_callbacks_admit_only_the_requested_zeroed_memory() {
    STATIC_BYTES.with(|bytes| bytes.set(16));
    STATIC_CALLS.with(|calls| calls.set(0));
    let mut req = NtStaticRequirements { dram: 999 };
    let mut bytes = [0xa5; 18];
    let mut memory = NtStaticMemoryPtrs {
        dram: unsafe { bytes.as_mut_ptr().add(1) },
    };
    unsafe {
        (Bridge::FACTORY.calculate_static_requirements.unwrap())(&mut req);
        (Bridge::FACTORY.initialise.unwrap())(&mut memory, &req);
    }
    assert_eq!(req.dram, 16);
    assert_eq!(STATIC_CALLS.with(Cell::get), 1);
    assert!(STATIC_ZEROED.with(Cell::get));
    assert_eq!((bytes[0], bytes[17]), (0xa5, 0xa5));
    for index in 0..16 {
        assert_eq!(bytes[index + 1], index as u8 ^ 0x5a);
    }
}

#[test]
fn static_initialise_handles_empty_allocations_and_rejects_null_nonempty_memory() {
    STATIC_CALLS.with(|calls| calls.set(0));
    STATIC_BYTES.with(|bytes| bytes.set(0));
    let mut req = NtStaticRequirements { dram: 999 };
    let mut memory = NtStaticMemoryPtrs {
        dram: ptr::null_mut(),
    };
    unsafe {
        (Bridge::FACTORY.calculate_static_requirements.unwrap())(&mut req);
        (Bridge::FACTORY.initialise.unwrap())(&mut memory, &req);
    }
    assert_eq!(req.dram, 0);
    assert_eq!(STATIC_CALLS.with(Cell::get), 1);
    STATIC_BYTES.with(|bytes| bytes.set(16));
    unsafe {
        (Bridge::FACTORY.calculate_static_requirements.unwrap())(&mut req);
        (Bridge::FACTORY.initialise.unwrap())(&mut memory, &req);
    }
    assert_eq!(STATIC_CALLS.with(Cell::get), 1);
}

#[test]
fn zero_parameters_accept_a_null_host_value_pointer() {
    let mut allocation = Allocation::new();
    MODE.with(|mode| mode.set(Mode::NoParameters));
    unsafe {
        Bridge::calculate_requirements(&mut allocation.req, ptr::null());
    }
    let base = allocation.construct();
    assert!(!base.is_null());
    assert!(unsafe { (*base).v }.is_null());
    let mut context = unsafe { Context::<u32, u32, Audio>::admit(base) };
    let data = context.processing();
    assert!(data.parameters.is_empty() && data.voices.is_empty());
    assert_eq!(*data.state, 123);
}

#[test]
fn text_callbacks_cannot_report_out_of_buffer_lengths() {
    for bad in [-1, 16, i32::MAX] {
        let mut bytes = [1; 16];
        assert_eq!(finish_text(&mut bytes, bad), 0);
        assert_eq!(bytes[0], 0);
    }
    let mut bytes = [1; 16];
    assert_eq!(finish_text(&mut bytes, 15), 15);
    assert_eq!(bytes[15], 0);
}

#[no_mangle]
unsafe extern "C" fn rust_nt_globals() -> *const NtGlobals {
    static GLOBALS: NtGlobals = NtGlobals {
        sample_rate: 48000,
        max_frames_per_step: 128,
        work_buffer: ptr::null_mut(),
        work_buffer_size_bytes: 0,
        stream_size_bytes: 0,
        stream_buffer_size_bytes: 0,
    };
    &GLOBALS
}

struct Allocation {
    raw: *mut u8,
    layout: Layout,
    req: NtAlgorithmRequirements,
}
impl Allocation {
    fn new() -> Self {
        Self::for_plugin::<Example>()
    }
    fn for_plugin<P: Plugin>() -> Self {
        MODE.with(|mode| mode.set(Mode::Valid));
        let mut req = NtAlgorithmRequirements {
            num_parameters: 0,
            sram: 0,
            dram: 0,
            dtc: 0,
            itc: 0,
        };
        // SAFETY: this is the native host fixture, supplying the exact callback ABI.
        unsafe {
            Registration::<P, 0>::calculate_requirements(&mut req, ptr::null());
        }
        let layout = Layout::from_size_align(
            req.sram as usize + Registration::<P, 0>::alignment(&[]).unwrap(),
            Registration::<P, 0>::alignment(&[]).unwrap(),
        )
        .unwrap();
        let raw = unsafe { alloc(layout) };
        assert!(!raw.is_null());
        unsafe {
            ptr::write_bytes(raw, 0xa5, layout.size());
        }
        Self { raw, layout, req }
    }
    fn construct(&self) -> *mut NtAlgorithm {
        self.construct_plugin::<Example>()
    }
    fn construct_plugin<P: Plugin>(&self) -> *mut NtAlgorithm {
        let mem = NtAlgorithmMemoryPtrs {
            sram: self.raw,
            dram: ptr::null_mut(),
            dtc: ptr::null_mut(),
            itc: ptr::null_mut(),
        };
        unsafe { (Registration::<P, 0>::FACTORY.construct.unwrap())(&mem, &self.req, ptr::null()) }
    }
}
impl Drop for Allocation {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.raw, self.layout);
        }
        MODE.with(|mode| mode.set(Mode::Valid));
    }
}

#[test]
fn poisoned_host_storage_is_initialized_and_metadata_is_owned() {
    let allocation = Allocation::new();
    let base = allocation.construct();
    assert!(!base.is_null());
    let values = [7i16];
    unsafe {
        (*base).v = values.as_ptr();
    }
    let mut context = unsafe { Context::<u32, u32, Audio>::admit(base) };
    assert_eq!(*context.state(), 123);
    assert_eq!(context.voices(), &[0, 1]);
    assert_eq!(context.read_parameters().value(0), Some(7));
    {
        let data = context.processing();
        *data.state = 321;
        data.voices[1] = 99;
        assert_eq!(data.parameters.value(0), Some(7));
        assert_eq!(data.parameters.len(), 1);
    }
    assert_eq!(*context.state(), 321);
    assert_eq!(context.voices(), &[0, 99]);
    unsafe {
        let pages = &*(*base).parameter_pages;
        assert_eq!(pages.num_pages, 1);
        assert_eq!(CStr::from_ptr((*pages.pages).name), c"Test");
        assert_eq!(*(*pages.pages).params, 0);
    }
    assert_eq!(
        (allocation.req.dram, allocation.req.dtc, allocation.req.itc),
        (0, 0, 0)
    );
}

#[test]
fn allocation_regions_are_aligned_disjoint_and_within_the_requirement() {
    let config = Example::configuration(&[]).unwrap();
    let layout = InstanceLayout::new::<u32, u32, Sram>(config).unwrap();
    let regions = [
        (
            0,
            core::mem::size_of::<Storage<u32, Sram>>(),
            core::mem::align_of::<Storage<u32, Sram>>(),
        ),
        (
            layout.parameters,
            core::mem::size_of::<NtParameter>() * config.parameters,
            core::mem::align_of::<NtParameter>(),
        ),
        (
            layout.pages,
            core::mem::size_of::<NtParameterPage>() * config.pages,
            core::mem::align_of::<NtParameterPage>(),
        ),
        (layout.indices, config.page_indices, 1),
        (
            layout.voices,
            core::mem::size_of::<u32>() * config.voices,
            core::mem::align_of::<u32>(),
        ),
    ];
    let mut end = 0;
    for (start, size, align) in regions {
        assert!(start >= end);
        assert_eq!(start % align, 0);
        end = start + size;
    }
    assert_eq!(end, layout.allocation.size());
    for config in [
        Configuration {
            parameters: 257,
            ..config
        },
        Configuration {
            voices: usize::MAX,
            ..config
        },
    ] {
        assert!(InstanceLayout::new::<u32, u32, Sram>(config).is_none());
    }
}

#[test]
fn mismatched_counts_short_and_misaligned_allocations_fail_before_writing() {
    let mut allocation = Allocation::new();
    allocation.req.num_parameters += 1;
    assert!(allocation.construct().is_null());
    allocation.req.num_parameters -= 1;
    allocation.req.sram -= 1;
    assert!(allocation.construct().is_null());
    allocation.req.sram += 1;
    let mut mem = NtAlgorithmMemoryPtrs {
        sram: unsafe { allocation.raw.add(1) },
        dram: ptr::null_mut(),
        dtc: ptr::null_mut(),
        itc: ptr::null_mut(),
    };
    assert!(unsafe { Bridge::construct(&mem, &allocation.req, ptr::null()) }.is_null());
    mem.sram = ptr::null_mut();
    assert!(unsafe { Bridge::construct(&mem, &allocation.req, ptr::null()) }.is_null());
    assert!(
        unsafe { slice::from_raw_parts(allocation.raw, allocation.layout.size()) }
            .iter()
            .all(|&byte| byte == 0xa5)
    );
}

#[test]
fn failed_safe_plugin_initialization_is_never_published() {
    for mode in [
        Mode::MissingParameter,
        Mode::BadPage,
        Mode::BadState,
        Mode::BadVoice,
        Mode::TooManyParameters,
    ] {
        let allocation = Allocation::new();
        MODE.with(|current| current.set(mode));
        assert!(allocation.construct().is_null());
    }
}

#[test]
fn metadata_rejects_holes_bad_indices_and_capacity_overruns() {
    let mut parameters = [MaybeUninit::uninit(); 2];
    let mut pages = [MaybeUninit::uninit(); 1];
    let mut indices = [MaybeUninit::uninit(); 2];
    let mut metadata = Metadata {
        parameters: &mut parameters,
        pages: &mut pages,
        indices: &mut indices,
        initialized: [0; 8],
        initialized_count: 0,
        page_count: 0,
        used: 0,
    };
    let definition = Definition::new(c"Test", 0, 1, 0, UNIT_NONE, SCALING_NONE, None).unwrap();
    assert!(metadata.set_parameter(2, definition).is_none());
    metadata.set_parameter(0, definition).unwrap();
    metadata.set_parameter(0, definition).unwrap();
    assert!(metadata.finish().is_none());
    metadata.set_parameter(1, definition).unwrap();
    assert!(metadata.add_page(c"Bad", 0, &[2]).is_none());
    assert!(metadata.add_page(c"Empty", 0, &[]).is_none());
    assert!(metadata.add_page(c"Too long", 0, &[0; 256]).is_none());
    assert!(metadata
        .add_page(c"Too many indices", 0, &[0, 1, 0])
        .is_none());
    metadata.add_page(c"Valid", 1, &[0, 1]).unwrap();
    assert_eq!(metadata.finish(), Some(1));
    assert!(metadata.add_page(c"Extra page", 1, &[0]).is_none());
}

#[test]
fn definition_construction_validates_scalar_bounds_enum_capacity_and_io_ranges() {
    static ENUMS: EnumStrings<2> = EnumStrings::new([c"No", c"Yes"]);
    assert!(Definition::new(c"Bad default", 0, 1, 2, UNIT_NONE, 0, None).is_none());
    assert!(Definition::new(c"Bad scaling", 0, 1, 0, UNIT_NONE, 4, None).is_none());
    assert!(Definition::new(c"No table", 0, 1, 0, UNIT_ENUM, 0, None).is_none());
    assert!(Definition::new(c"Short table", 0, 2, 0, UNIT_ENUM, 0, Some(ENUMS.table())).is_none());
    assert!(Definition::new(
        c"Negative enum",
        -1,
        1,
        0,
        UNIT_ENUM,
        0,
        Some(ENUMS.table())
    )
    .is_none());
    assert!(Definition::new(c"Bad bus", 0, 65, 0, UNIT_CV_OUTPUT, 0, None).is_none());
    assert!(Definition::new(c"Good table", 0, 1, 0, UNIT_ENUM, 0, Some(ENUMS.table())).is_some());
    assert_eq!(
        core::mem::size_of::<Definition>(),
        core::mem::size_of::<NtParameter>()
    );
}
