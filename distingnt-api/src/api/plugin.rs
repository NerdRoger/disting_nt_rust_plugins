//! Allocation-free plugin framework. All ABI and host-memory admission stays here.
//!
//! A context is neither Clone nor Copy. State/region borrows and parameter setter
//! capabilities borrow that same context, so safe code cannot keep a state borrow
//! across a setter that synchronously reenters the plugin.
use super::{
    drawing::Canvas,
    json::{JsonReader, JsonWriter},
    metadata::Definition,
    parameters::{Audio, ParameterValues, Parameters, ReadOnly, Ui},
};

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod tests;
use super::state_memory::{Placement, Sram, StateMemory};
use crate::raw::*;
use core::{
    alloc::Layout,
    ffi::{c_char, CStr},
    marker::PhantomData,
    mem::MaybeUninit,
    ptr, slice,
};

#[derive(Clone, Copy)]
pub struct Configuration {
    pub parameters: usize,
    pub pages: usize,
    pub page_indices: usize,
    pub regions: usize,
}

/// Factory-wide DRAM requested during scan/loading, shared by all instances.
#[derive(Clone, Copy, Default)]
pub struct StaticRequirements {
    pub dram: u32,
}

/// Zero-initialized host allocation, borrowed only during initialise.
/// The host owns its lifetime; this view cannot be retained by safe plugin code.
/// ```compile_fail
/// use distingnt_api::api::plugin::StaticMemory;
/// fn retain(memory: StaticMemory<'_>) -> &'static mut [u8] {
///     memory.dram
/// }
/// ```
pub struct StaticMemory<'a> {
    pub dram: &'a mut [u8],
}

pub struct Specification {
    pub name: &'static CStr,
    pub min: i32,
    pub max: i32,
    pub default: i32,
    pub kind: i32,
}

/// Exclusive initializer for a preallocated region of RAM holding an object.
/// Use [`crate::initialize_region!`] to write struct fields without a whole-state
/// temporary. Scalars use `set`; arrays use `fill` or `fill_with`. Success without
/// initializing the destination makes construction fail safely.
///
/// Whole-struct setters are deliberately unavailable:
/// ```compile_fail
/// use distingnt_api::api::plugin::RegionInitializer;
/// #[derive(Clone, Copy)]
/// struct State { data: [u32; 1024] }
/// fn aggregate_setter(destination: &mut RegionInitializer<'_, State>) {
///     destination.set(State { data: [0; 1024] });
/// }
/// ```
pub struct RegionInitializer<'a, S: Copy> {
    destination: *mut S,
    initialized: bool,
    lifetime: PhantomData<&'a mut MaybeUninit<S>>,
}

impl<S: Copy> RegionInitializer<'_, S> {
    // SAFETY: caller supplies exclusive aligned storage for S for this lifetime.
    unsafe fn new(destination: *mut S) -> Self {
        Self {
            destination,
            initialized: false,
            lifetime: PhantomData,
        }
    }

    fn write_value(&mut self, value: S) {
        // SAFETY: the framework owns the destination; S has no Drop resources.
        unsafe {
            self.destination.write(value);
        }
        self.initialized = true;
    }

    #[doc(hidden)]
    pub fn __destination(&mut self) -> *mut S {
        self.destination
    }

    /// # Safety
    /// Every field of the destination must have been initialized with a valid
    /// value. Used by initialize_region! after its exhaustive field writes.
    #[doc(hidden)]
    pub unsafe fn __finish(&mut self) {
        self.initialized = true;
    }
}

// Only scalar values expose a value-taking setter; aggregates must use the
// exhaustive field macro or element-by-element array initializer below.
macro_rules! scalar_initializers {
    ($($scalar:ty),* $(,)?) => {$(
        impl RegionInitializer<'_, $scalar> {
            pub fn set(&mut self, value: $scalar) { self.write_value(value); }
        }
    )*};
}
scalar_initializers!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64
);

impl<T: Copy, const N: usize> RegionInitializer<'_, [T; N]> {
    /// Initialize aggregate elements in their final locations. Unlike fill_with,
    /// the callback never returns an element by value. Any unfinished element
    /// leaves the entire array unpublished.
    pub fn initialize_each(
        &mut self,
        mut initialize: impl FnMut(&mut RegionInitializer<'_, T>, usize),
    ) {
        for index in 0..N {
            // SAFETY: the owned array contains N exclusive, aligned T slots.
            let mut element =
                unsafe { RegionInitializer::new(self.destination.cast::<T>().add(index)) };
            initialize(&mut element, index);
            if !element.initialized {
                return;
            }
        }
        self.initialized = true;
    }

    /// Initialize array elements directly, without creating a temporary array.
    pub fn fill(&mut self, value: T) {
        self.fill_with(|_| value);
    }
    pub fn fill_with(&mut self, mut value: impl FnMut(usize) -> T) {
        for index in 0..N {
            // SAFETY: framework owns an aligned allocation for all N elements.
            unsafe {
                self.destination.cast::<T>().add(index).write(value(index));
            }
        }
        self.initialized = true;
    }
}

/// Initialize a struct directly in a preallocated region of RAM. All fields
/// must be listed exactly once, as in an ordinary struct literal. The API owns
/// the raw writes; callers can continue to forbid unsafe code.
///
/// ```
/// #![forbid(unsafe_code)]
/// use distingnt_api::api::plugin::RegionInitializer;
/// #[derive(Clone, Copy)]
/// struct State { a: u32, b: bool }
/// fn initialize(destination: &mut RegionInitializer<'_, State>) {
///     distingnt_api::initialize_region!(destination, State { a: 42, b: true });
/// }
/// ```
///
/// ```compile_fail
/// use distingnt_api::api::plugin::RegionInitializer;
/// #[derive(Clone, Copy)]
/// struct State { a: u32, b: u32 }
/// fn incomplete(destination: &mut RegionInitializer<'_, State>) {
///     distingnt_api::initialize_region!(destination, State { a: 1 });
/// }
/// ```
///
/// ```compile_fail
/// #![forbid(unsafe_code)]
/// #[derive(Clone, Copy)]
/// struct State { a: u32 }
/// struct Fake;
/// impl Fake {
///     fn __destination(&mut self) -> *mut State { core::ptr::null_mut() }
///     fn __finish(&mut self) {}
/// }
/// fn forged_destination(fake: &mut Fake) {
///     distingnt_api::initialize_region!(fake, State { a: 1 });
/// }
/// ```
/// Array fields can be filled element by element without a temporary array:
/// ```
/// #![forbid(unsafe_code)]
/// use distingnt_api::api::plugin::RegionInitializer;
/// #[derive(Clone, Copy)]
/// struct State { flag: bool, samples: [u32; 1024] }
/// fn initialize(destination: &mut RegionInitializer<'_, State>) {
///     distingnt_api::initialize_region!(destination, State { flag: true },
///         arrays { samples: |index| index as u32 });
/// }
/// ```
/// Missing fields are still rejected across both groups:
/// ```compile_fail
/// use distingnt_api::api::plugin::RegionInitializer;
/// #[derive(Clone, Copy)]
/// struct State { flag: bool, missing: u8, samples: [u32; 4] }
/// fn initialize(destination: &mut RegionInitializer<'_, State>) {
///     distingnt_api::initialize_region!(destination, State { flag: true },
///         arrays { samples: |_| 0 });
/// }
/// ```
/// Nested aligned fields can use their own initializer rather than return an
/// aggregate by value. A nested initializer that does not finish leaves the
/// parent incomplete, so construction cannot publish it. Unaligned nested
/// fields in packed structs are rejected before their initializer is called.
/// ```
/// #![forbid(unsafe_code)]
/// use distingnt_api::api::plugin::RegionInitializer;
/// #[derive(Clone, Copy)]
/// struct Child { values: [u32; 32] }
/// #[derive(Clone, Copy)]
/// struct Parent { child: Child, flag: bool }
/// fn initialize(destination: &mut RegionInitializer<'_, Parent>) {
///     distingnt_api::initialize_region!(destination, Parent { flag: true }, arrays {},
///         regions { child: |child| {
///             distingnt_api::initialize_region!(child, Child {},
///                 arrays { values: |index| index as u32 });
///         }});
/// }
/// ```
#[macro_export]
macro_rules! initialize_region {
    ($destination:expr, $state:ident { $($field:ident: $value:expr),* $(,)? }) => {
        $crate::initialize_region!($destination, $state { $($field: $value),* }, arrays {}, regions {})
    };
    ($destination:expr, $state:ident { $($field:ident: $value:expr),* $(,)? },
        arrays { $($array:ident: $element:expr),* $(,)? }) => {
        $crate::initialize_region!($destination, $state { $($field: $value),* },
            arrays { $($array: $element),* }, regions {})
    };
    ($destination:expr, $state:ident { $($field:ident: $value:expr),* $(,)? },
        arrays { $($array:ident: $element:expr),* $(,)? },
        regions { $($region:ident: $initialize:expr),* $(,)? }) => {{
        // Check omissions/duplicates across all groups without building S.
        let _ = |value: $state| { let $state { $($field: _,)* $($array: _,)* $($region: _,)* } = value; };
        let initializer: &mut $crate::api::plugin::RegionInitializer<'_, $state> = &mut *$destination;
        let pointer: *mut $state = initializer.__destination();
        // SAFETY: typed exclusive destination, every field checked above, each
        // expression yields that field's type. Failure/panic never publishes it.
        $(unsafe { ::core::ptr::addr_of_mut!((*pointer).$field).write_unaligned($value); })*
        $(unsafe { $crate::api::plugin::__initialize_array_field(
            ::core::ptr::addr_of_mut!((*pointer).$array), $element); })*
        let complete = true $(& {
            let destination = unsafe { ::core::ptr::addr_of_mut!((*pointer).$region) };
            let initialize = $crate::api::plugin::__nested_initializer(destination, $initialize);
            unsafe { $crate::api::plugin::__initialize_nested_region(destination, initialize) }
        })*;
        if complete {
            unsafe { initializer.__finish(); }
        }
    }};
}

// Give closures their higher-ranked callback type outside the unsafe block.
#[doc(hidden)]
#[inline(always)]
pub fn __nested_initializer<T: Copy, F: FnOnce(&mut RegionInitializer<'_, T>)>(
    _: *mut T,
    initialize: F,
) -> F {
    initialize
}

/// # Safety
/// The destination is exclusive writable storage for T. Alignment is checked
/// before admitting the nested initializer; unaligned packed fields fail safely.
#[doc(hidden)]
#[inline(always)]
pub unsafe fn __initialize_nested_region<T: Copy>(
    destination: *mut T,
    initialize: impl FnOnce(&mut RegionInitializer<'_, T>),
) -> bool {
    if !(destination as usize).is_multiple_of(core::mem::align_of::<T>()) {
        return false;
    }
    // SAFETY: exclusive writable destination supplied by the macro, now aligned.
    let mut nested = unsafe { RegionInitializer::new(destination) };
    initialize(&mut nested);
    nested.initialized
}

/// # Safety
/// The destination must be exclusively writable storage for the array. It may
/// be unaligned (a packed field). Called only by the exhaustive initializer macro.
#[doc(hidden)]
#[inline(always)]
pub unsafe fn __initialize_array_field<T: Copy, const N: usize>(
    destination: *mut [T; N],
    mut element: impl FnMut(usize) -> T,
) {
    for index in 0..N {
        // SAFETY: the caller owns all N elements; index is within that allocation.
        // No reference is formed, so a packed field need not be aligned for T.
        unsafe {
            destination
                .cast::<T>()
                .add(index)
                .write_unaligned(element(index));
        }
    }
}

/// Callback state contains values only: no destructor is silently lost when the
/// C++ host reclaims SRAM. Copy is a constraint, not a per-callback copying policy.
/// Both direct-initialization hooks are mandatory. A plugin cannot rely on
/// a value-returning construction fallback:
/// ```compile_fail
/// use distingnt_api::api::plugin::{Plugin, Configuration, Metadata, Specification};
/// use distingnt_api::api::state_memory::Sram;
/// struct MissingInitializers;
/// impl Plugin for MissingInitializers {
///     type State = ();
///     type Region = ();
///     type StateMemory = Sram;
///     const GUID: u32 = 1;
///     const NAME: &'static core::ffi::CStr = c"Missing initializers";
///     const DESCRIPTION: &'static core::ffi::CStr = c"";
///     const TAGS: u32 = 0;
///     const SPECIFICATIONS: &'static [Specification] = &[];
///     fn configuration(_: &[i32]) -> Option<Configuration> { None }
///     fn metadata(_: &[i32], _: &mut Metadata<'_>) -> Option<()> { Some(()) }
/// }
/// ```
pub trait Plugin: Sized + 'static {
    type State: Copy + 'static;
    type Region: Copy + 'static;
    type StateMemory: StateMemory<Self::State>;
    const GUID: u32;
    const NAME: &'static CStr;
    const DESCRIPTION: &'static CStr;
    const TAGS: u32;
    const SPECIFICATIONS: &'static [Specification];
    const CUSTOM_UI: u32 = 0;
    const PRESETS: bool = false;
    const PARAMETER_TEXT: bool = false;
    const MIDI: bool = false;
    /// Factory-wide SysEx reception, independent of per-instance MIDI callbacks.
    const MIDI_SYSEX: bool = false;
    /// Expose plugin-owned editable buffers for UNIT_STRING parameters.
    const STRING_PARAMETERS: bool = false;
    /// Enable the optional factory-wide requirements and initialise callbacks.
    const STATIC_INITIALIZATION: bool = false;
    /// Place requirements calculation and instance construction in DRAM.
    /// Exclusive callees follow when packaging with --dram-callees.
    const INITIALIZATION_IN_DRAM: bool = false;
    /// Place drawing and its exclusive callees in DRAM when packaging with --dram-callees.
    const DRAWING_IN_DRAM: bool = false;
    /// Place preset serialization/deserialization callbacks in DRAM.
    /// Exclusive callees follow when packaging with --dram-callees.
    const SERIALIZATION_IN_DRAM: bool = false;

    fn calculate_static_requirements() -> StaticRequirements {
        StaticRequirements::default()
    }
    fn initialise(_: StaticMemory<'_>) {}

    fn configuration(specifications: &[i32]) -> Option<Configuration>;
    /// Initialize state directly in its final allocation. Required for every
    /// plugin; use initialize_region! for structs, set for scalars, fill for arrays.
    fn initialize_state(
        destination: &mut RegionInitializer<'_, Self::State>,
        specifications: &[i32],
        sample_rate: u32,
    ) -> Option<()>;
    /// Initialize one preallocated region in its final SRAM allocation.
    fn initialize_region(
        destination: &mut RegionInitializer<'_, Self::Region>,
        specifications: &[i32],
        index: usize,
    ) -> Option<()>;
    fn metadata(specifications: &[i32], metadata: &mut Metadata<'_>) -> Option<()>;
    fn parameter_changed(
        _: &mut Context<'_, Self::State, Self::Region, Audio, Self::StateMemory>,
        _: i32,
    ) {
    }
    fn step(
        _: &mut Context<'_, Self::State, Self::Region, Audio, Self::StateMemory>,
        _: &mut [f32],
        _: usize,
    ) {
    }
    fn draw(
        _: &mut Context<'_, Self::State, Self::Region, Ui, Self::StateMemory>,
        _: Canvas<'_>,
    ) -> bool {
        false
    }
    fn custom_ui(
        _: &mut Context<'_, Self::State, Self::Region, Ui, Self::StateMemory>,
        _: &NtUiData,
    ) {
    }
    /// Return the currently overridden controls, within the CUSTOM_UI mask.
    fn has_custom_ui(
        _: &mut Context<'_, Self::State, Self::Region, ReadOnly, Self::StateMemory>,
    ) -> u32 {
        Self::CUSTOM_UI
    }
    fn setup_ui(_: &mut Context<'_, Self::State, Self::Region, Ui, Self::StateMemory>) -> NtFloat3 {
        [0.5; 3]
    }
    fn serialise(
        _: &mut Context<'_, Self::State, Self::Region, ReadOnly, Self::StateMemory>,
        _: &mut JsonWriter<'_>,
    ) {
    }
    fn deserialise(
        _: &mut Context<'_, Self::State, Self::Region, ReadOnly, Self::StateMemory>,
        _: &mut JsonReader<'_>,
    ) -> bool {
        true
    }
    fn prefix(
        _: &mut Context<'_, Self::State, Self::Region, ReadOnly, Self::StateMemory>,
        _: i32,
        _: &mut [c_char],
    ) -> i32 {
        0
    }
    fn parameter_string(
        _: &mut Context<'_, Self::State, Self::Region, ReadOnly, Self::StateMemory>,
        _: i32,
        _: i32,
        _: &mut [c_char],
    ) -> i32 {
        0
    }
    fn midi_realtime(
        _: &mut Context<'_, Self::State, Self::Region, Audio, Self::StateMemory>,
        _: u8,
    ) {
    }
    fn midi_message(
        _: &mut Context<'_, Self::State, Self::Region, Audio, Self::StateMemory>,
        _: u8,
        _: u8,
        _: u8,
    ) {
    }
    /// The C++ host supplies no instance for this factory-wide callback.
    fn midi_sysex(_: &[u8]) {}

    /// Return a persistent, initialized byte buffer, including space for NUL.
    /// The host accesses it synchronously and never retains its pointer.
    /// Return None for unsupported parameters; empty buffers are also rejected.
    /// The API terminates the final byte before handing the buffer to the host.
    /// Bytes need not be UTF-8, so host edits cannot invalidate a Rust string.
    /// ```compile_fail
    /// use distingnt_api::api::{plugin::{Plugin, Context}, parameters::ReadOnly};
    /// fn retain<P: Plugin>(
    ///     context: &mut Context<'_, P::State, P::Region, ReadOnly, P::StateMemory>,
    /// ) -> &'static mut [u8] {
    ///     P::string_parameter_buffer(context, 0).unwrap()
    /// }
    /// ```
    fn string_parameter_buffer<'a>(
        _: &'a mut Context<'_, Self::State, Self::Region, ReadOnly, Self::StateMemory>,
        _: i32,
    ) -> Option<&'a mut [u8]> {
        None
    }
}

/// The fields are private: plugin code cannot forge callback state or counts.
#[repr(C)]
struct Storage<S: Copy, M: StateMemory<S>> {
    base: NtAlgorithm,
    pages: NtParameterPages,
    parameter_count: usize,
    region_count: usize,
    region_offset: usize,
    state: M::Slot,
}

pub struct Context<'callback, S: Copy, V: Copy, Mode, M: StateMemory<S> = Sram> {
    raw: *mut Storage<S, M>,
    _lifetime: PhantomData<&'callback mut (S, V, Mode)>,
}
pub struct Processing<'a, S, V> {
    pub state: &'a mut S,
    pub regions: &'a mut [V],
    pub parameters: ParameterValues<'a>,
}
impl<S: Copy, V: Copy, Mode, M: StateMemory<S>> Context<'_, S, V, Mode, M> {
    /// ABI admission for external host harnesses. Normal plugins receive contexts
    /// from Registration and never call this constructor.
    /// # Safety
    /// base is a live instance constructed by the matching Registration with
    /// these exact S/V types and state-memory policy M. Its v array and all
    /// selected state-memory regions remain initialized and live. The
    /// context remains inside the callback, and no incompatible borrow coexists.
    pub(crate) unsafe fn admit(base: *mut NtAlgorithm) -> Self {
        Self {
            raw: base.cast(),
            _lifetime: PhantomData,
        }
    }
    /// Native host-test admission; excluded from the production facade.
    /// # Safety
    /// The pointer is a matching live framework instance; all host obligations
    /// from the framework safety contract apply.
    #[cfg(feature = "host-test")]
    pub unsafe fn from_raw(base: *mut NtAlgorithm) -> Self {
        unsafe { Self::admit(base) }
    }
    /// Current zero-based host algorithm index, if this instance is in a slot.
    pub fn algorithm_index(&self) -> Option<u32> {
        // SAFETY: this callback capability admits a live matching host instance.
        u32::try_from(unsafe { NT_algorithmIndex(self.raw.cast()) }).ok()
    }
    #[inline]
    pub fn state(&self) -> &S {
        // SAFETY: registration initializes state; this borrow freezes the context.
        unsafe { &*M::pointer(ptr::addr_of_mut!((*self.raw).state)) }
    }
    #[inline]
    pub fn state_mut(&mut self) -> &mut S {
        // SAFETY: this exclusive context borrow excludes other safe state access.
        unsafe { &mut *M::pointer(ptr::addr_of_mut!((*self.raw).state)) }
    }
    fn region_pointer(&self) -> *mut V {
        // SAFETY: the checked layout stores an aligned in-allocation region offset.
        unsafe { self.raw.cast::<u8>().add((*self.raw).region_offset).cast() }
    }
    #[inline]
    pub fn regions(&self) -> &[V] {
        // SAFETY: registration initializes exactly region_count elements.
        unsafe { slice::from_raw_parts(self.region_pointer(), (*self.raw).region_count) }
    }
    #[inline]
    pub fn regions_mut(&mut self) -> &mut [V] {
        // SAFETY: the exclusive context borrow excludes all aliases to this region.
        unsafe { slice::from_raw_parts_mut(self.region_pointer(), (*self.raw).region_count) }
    }
    /// Borrows state and regions together without calling or borrowing the host.
    #[inline]
    pub fn processing(&mut self) -> Processing<'_, S, V> {
        // SAFETY: state, region storage, definitions and host v are disjoint.
        // The exclusive context borrow prevents creating a setter until these
        // references have expired; these references cannot escape the callback.
        unsafe {
            Processing {
                state: &mut *M::pointer(ptr::addr_of_mut!((*self.raw).state)),
                regions: slice::from_raw_parts_mut(self.region_pointer(), (*self.raw).region_count),
                parameters: ParameterValues::new(
                    if (*self.raw).parameter_count == 0 {
                        &[]
                    } else {
                        slice::from_raw_parts((*self.raw).base.v, (*self.raw).parameter_count)
                    },
                    if (*self.raw).parameter_count == 0 {
                        &[]
                    } else {
                        slice::from_raw_parts(
                            (*self.raw).base.parameters,
                            (*self.raw).parameter_count,
                        )
                    },
                ),
            }
        }
    }
    #[inline]
    pub fn read_parameters(&self) -> Parameters<'_, ReadOnly> {
        // SAFETY: matching initialized instance; the returned handle borrows self.
        unsafe {
            Parameters::from_raw(
                self.raw.cast(),
                (*self.raw).base.parameters.cast_mut(),
                (*self.raw).parameter_count,
            )
        }
    }
    /// The handle's lifetime borrows this context exclusively, preventing state
    /// references from spanning a reentrant setter/notification.
    /// ```compile_fail
    /// use distingnt_api::api::{plugin::Context, parameters::Audio};
    /// fn invalid(context: &mut Context<'_, u32, (), Audio>) {
    ///     let state = context.state_mut();
    ///     context.parameters().set(0, 1);
    ///     *state = 2;
    /// }
    /// ```
    #[inline]
    pub fn parameters(&mut self) -> Parameters<'_, Mode> {
        // SAFETY: the ABI bridge established Mode for this callback. Exclusive
        // borrowing of self prevents any instance references across host calls.
        unsafe {
            Parameters::from_raw(
                self.raw.cast(),
                (*self.raw).base.parameters.cast_mut(),
                (*self.raw).parameter_count,
            )
        }
    }
}

impl<S: Copy, V: Copy, M: StateMemory<S>> Context<'_, S, V, Audio, M> {
    /// Send a complete SysEx message from an audio callback. Include F0 and
    /// seven-bit payload bytes; the host appends F7 synchronously.
    pub fn send_midi_sysex(&mut self, destination: u32, message: &[u8]) -> bool {
        if destination == 0
            || destination & !(DESTINATION_INTERNAL | DESTINATION_BREAKOUT | DESTINATION_USB) != 0
            || message.first() != Some(&0xf0)
            || message[1..].iter().any(|&byte| byte >= 0x80)
        {
            return false;
        }
        let Ok(count) = u32::try_from(message.len()) else {
            return false;
        };
        // SAFETY: audio callback capability, validated MIDI data and synchronous
        // host consumption of this borrowed byte buffer.
        unsafe {
            NT_sendMidiSysEx(destination, message.as_ptr(), count, true);
        }
        true
    }
}

/// Construction-only builder; only the framework can create it. Safe plugins
/// cannot replace its buffers or publish uninitialized/raw pointer metadata.
pub struct Metadata<'a> {
    parameters: &'a mut [MaybeUninit<NtParameter>],
    pages: &'a mut [MaybeUninit<NtParameterPage>],
    indices: &'a mut [MaybeUninit<u8>],
    initialized: [u32; 8],
    initialized_count: usize,
    page_count: usize,
    used: usize,
}
impl Metadata<'_> {
    pub fn parameter_count(&self) -> usize {
        self.parameters.len()
    }
    pub fn set_parameter(&mut self, index: usize, definition: Definition) -> Option<()> {
        let raw = definition.raw();
        self.parameters.get_mut(index)?.write(raw);
        let word = self.initialized.get_mut(index / 32)?;
        let mask = 1u32 << (index % 32);
        if *word & mask == 0 {
            *word |= mask;
            self.initialized_count += 1;
        }
        Some(())
    }
    pub fn add_page(&mut self, name: &'static CStr, group: u8, indices: &[u8]) -> Option<()> {
        self.add_named_page(super::metadata::Name::new(name), group, indices)
    }
    /// Add a page using a compact name from a static name table.
    pub fn add_named_page(
        &mut self,
        name: super::metadata::Name,
        group: u8,
        indices: &[u8],
    ) -> Option<()> {
        if indices.is_empty()
            || indices.len() > u8::MAX as usize
            || indices
                .iter()
                .any(|&index| index as usize >= self.parameters.len())
        {
            return None;
        }
        let end = self.used.checked_add(indices.len())?;
        let destination = self.indices.get_mut(self.used..end)?;
        for (slot, &index) in destination.iter_mut().zip(indices) {
            slot.write(index);
        }
        self.pages.get_mut(self.page_count)?.write(NtParameterPage {
            name: name.as_ptr(),
            num_params: indices.len() as u8,
            group,
            unused: [0; 2],
            params: destination.as_ptr().cast(),
        });
        self.page_count += 1;
        self.used = end;
        Some(())
    }
    fn finish(&self) -> Option<usize> {
        if self.initialized_count != self.parameters.len() {
            return None;
        }
        Some(self.page_count)
    }
}

struct InstanceLayout {
    allocation: Layout,
    parameters: usize,
    pages: usize,
    indices: usize,
    regions: usize,
}
impl InstanceLayout {
    fn new<S: Copy, V: Copy, M: StateMemory<S>>(config: Configuration) -> Option<Self> {
        if config.parameters > 256 || config.pages > u32::MAX as usize {
            return None;
        }
        let (layout, parameters) = Layout::new::<Storage<S, M>>()
            .extend(Layout::array::<NtParameter>(config.parameters).ok()?)
            .ok()?;
        let (layout, pages) = layout
            .extend(Layout::array::<NtParameterPage>(config.pages).ok()?)
            .ok()?;
        let (layout, indices) = layout
            .extend(Layout::array::<u8>(config.page_indices).ok()?)
            .ok()?;
        let (allocation, regions) = layout
            .extend(Layout::array::<V>(config.regions).ok()?)
            .ok()?;
        if allocation.size() > u32::MAX as usize || M::dtc_size() > u32::MAX as usize {
            return None;
        }
        Some(Self {
            allocation,
            parameters,
            pages,
            indices,
            regions,
        })
    }
}

pub struct Registration<P: Plugin, const N: usize>(PhantomData<P>);
impl<P: Plugin, const N: usize> Registration<P, N> {
    const SPECS: [NtSpecification; N] = {
        assert!(N == P::SPECIFICATIONS.len());
        let mut result = [NtSpecification {
            name: ptr::null(),
            min: 0,
            max: 0,
            default: 0,
            kind: 0,
        }; N];
        let mut i = 0;
        while i < N {
            let spec = &P::SPECIFICATIONS[i];
            assert!(spec.min <= spec.default && spec.default <= spec.max);
            assert!(spec.kind >= TYPE_GENERIC && spec.kind <= TYPE_BOOLEAN);
            result[i] = NtSpecification {
                name: spec.name.as_ptr(),
                min: spec.min,
                max: spec.max,
                default: spec.default,
                kind: spec.kind,
            };
            i += 1;
        }
        result
    };
    pub(crate) const FACTORY: NtFactory = NtFactory {
        guid: P::GUID,
        name: P::NAME.as_ptr(),
        description: P::DESCRIPTION.as_ptr(),
        tags: P::TAGS,
        num_specifications: N as u32,
        specifications: Self::SPECS.as_ptr(),
        calculate_static_requirements: if P::STATIC_INITIALIZATION {
            Some(if P::INITIALIZATION_IN_DRAM {
                Self::calculate_static_requirements_in_dram
            } else {
                Self::calculate_static_requirements
            })
        } else {
            None
        },
        initialise: if P::STATIC_INITIALIZATION {
            Some(if P::INITIALIZATION_IN_DRAM {
                Self::initialise_in_dram
            } else {
                Self::initialise
            })
        } else {
            None
        },
        calculate_requirements: Some(if P::INITIALIZATION_IN_DRAM {
            Self::calculate_requirements_in_dram
        } else {
            Self::calculate_requirements
        }),
        construct: Some(if P::INITIALIZATION_IN_DRAM {
            Self::construct_in_dram
        } else {
            Self::construct
        }),
        parameter_changed: Some(Self::parameter_changed),
        step: Some(Self::step),
        draw: Some(if P::DRAWING_IN_DRAM {
            Self::draw_in_dram
        } else {
            Self::draw
        }),
        midi_realtime: if P::MIDI {
            Some(Self::midi_realtime)
        } else {
            None
        },
        midi_message: if P::MIDI {
            Some(Self::midi_message)
        } else {
            None
        },
        has_custom_ui: if P::CUSTOM_UI != 0 {
            Some(Self::has_custom_ui)
        } else {
            None
        },
        custom_ui: if P::CUSTOM_UI != 0 {
            Some(Self::custom_ui)
        } else {
            None
        },
        setup_ui: if P::CUSTOM_UI != 0 {
            Some(Self::setup_ui)
        } else {
            None
        },
        serialise: if P::PRESETS {
            Some(if P::SERIALIZATION_IN_DRAM {
                Self::serialise_in_dram
            } else {
                Self::serialise
            })
        } else {
            None
        },
        deserialise: if P::PRESETS {
            Some(if P::SERIALIZATION_IN_DRAM {
                Self::deserialise_in_dram
            } else {
                Self::deserialise
            })
        } else {
            None
        },
        midi_sysex: if P::MIDI_SYSEX {
            Some(Self::midi_sysex)
        } else {
            None
        },
        parameter_ui_prefix: if P::PARAMETER_TEXT {
            Some(Self::prefix)
        } else {
            None
        },
        parameter_string: if P::PARAMETER_TEXT {
            Some(Self::parameter_string)
        } else {
            None
        },
        string_parameter_buffer: if P::STRING_PARAMETERS {
            Some(Self::string_parameter_buffer)
        } else {
            None
        },
    };
    pub fn entry(selector: u8, data: u32) -> usize {
        match selector {
            SELECTOR_VERSION => API_VERSION_CURRENT,
            SELECTOR_NUM_FACTORIES => 1,
            SELECTOR_FACTORY_INFO if data == 0 => &Self::FACTORY as *const NtFactory as usize,
            _ => 0,
        }
    }
    unsafe fn specs<'a>(raw: *const i32) -> Option<&'a [i32]> {
        let specs = if N == 0 {
            &[]
        } else {
            // SAFETY: the host supplies N readable specifications, per the factory.
            unsafe { slice::from_raw_parts(raw, N) }
        };
        if specs
            .iter()
            .zip(P::SPECIFICATIONS)
            .any(|(&value, spec)| value < spec.min || value > spec.max)
        {
            return None;
        }
        Some(specs)
    }
    pub fn alignment(specs: &[i32]) -> Option<usize> {
        Some(
            InstanceLayout::new::<P::State, P::Region, P::StateMemory>(P::configuration(specs)?)?
                .allocation
                .align(),
        )
    }
    crate::nt_dram_section! {
        /// # Safety
        /// Same live algorithm pointer requirements as draw.
        unsafe extern "C" fn draw_in_dram(base: *mut NtAlgorithm) -> bool {
            unsafe { Self::draw(base) }
        }
    }
    crate::nt_dram_section! {
        /// # Safety
        /// Same writable requirements argument as calculate_static_requirements.
        unsafe extern "C" fn calculate_static_requirements_in_dram(req: *mut NtStaticRequirements) {
            unsafe { Self::calculate_static_requirements(req); }
        }

        /// # Safety
        /// Same exclusive host allocation requirements as initialise.
        unsafe extern "C" fn initialise_in_dram(mem: *mut NtStaticMemoryPtrs, req: *const NtStaticRequirements) {
            unsafe { Self::initialise(mem, req); }
        }

        /// # Safety
        /// Same host argument requirements as calculate_requirements.
        unsafe extern "C" fn calculate_requirements_in_dram(
            req: *mut NtAlgorithmRequirements,
            specs: *const i32,
        ) {
            unsafe { Self::calculate_requirements(req, specs); }
        }

        /// # Safety
        /// Same host memory and argument requirements as construct.
        unsafe extern "C" fn construct_in_dram(
            mem: *const NtAlgorithmMemoryPtrs,
            req: *const NtAlgorithmRequirements,
            specs: *const i32,
        ) -> *mut NtAlgorithm {
            unsafe { Self::construct(mem, req, specs) }
        }

        /// # Safety
        /// Same live-instance and host-stream requirements as serialise.
        unsafe extern "C" fn serialise_in_dram(base: *mut NtAlgorithm, stream: *mut NtJsonStream) {
            unsafe { Self::serialise(base, stream); }
        }

        /// # Safety
        /// Same live-instance and host-parser requirements as deserialise.
        unsafe extern "C" fn deserialise_in_dram(base: *mut NtAlgorithm, parse: *mut NtJsonParse) -> bool {
            unsafe { Self::deserialise(base, parse) }
        }
    }

    /// # Safety
    /// The host provides a writable requirements record during scan/loading.
    #[inline(always)]
    pub(crate) unsafe extern "C" fn calculate_static_requirements(req: *mut NtStaticRequirements) {
        let requirements = P::calculate_static_requirements();
        unsafe {
            ptr::write(
                req,
                NtStaticRequirements {
                    dram: requirements.dram,
                },
            );
        }
    }

    /// # Safety
    /// mem/req are valid records; dram identifies an exclusive writable allocation
    /// of req.dram bytes. The host must not access it concurrently during this call.
    #[inline(always)]
    pub(crate) unsafe extern "C" fn initialise(
        mem: *mut NtStaticMemoryPtrs,
        req: *const NtStaticRequirements,
    ) {
        let bytes = unsafe { (*req).dram as usize };
        if bytes > isize::MAX as usize {
            return;
        }
        let dram = if bytes == 0 {
            &mut []
        } else {
            let pointer = unsafe { (*mem).dram };
            if pointer.is_null() {
                return;
            }
            // Initialize every byte before exposing a readable safe slice.
            unsafe {
                ptr::write_bytes(pointer, 0, bytes);
                slice::from_raw_parts_mut(pointer, bytes)
            }
        };
        P::initialise(StaticMemory { dram });
    }

    /// # Safety
    /// Called only by the C++ host with the factory's valid callback arguments.
    // Inline into the optional DRAM entry point so its body has that placement.
    #[inline(always)]
    pub(crate) unsafe extern "C" fn calculate_requirements(
        req: *mut NtAlgorithmRequirements,
        specs: *const i32,
    ) {
        let mut result = NtAlgorithmRequirements {
            num_parameters: 0,
            sram: 0,
            dram: 0,
            dtc: 0,
            itc: 0,
        };
        // SAFETY: host callback supplies readable specs and a writable result.
        if let Some((config, layout)) = unsafe { Self::specs(specs) }
            .and_then(P::configuration)
            .and_then(|config| {
                Some((
                    config,
                    InstanceLayout::new::<P::State, P::Region, P::StateMemory>(config)?,
                ))
            })
        {
            result.num_parameters = config.parameters as u32;
            result.sram = layout.allocation.size() as u32;
            result.dtc = P::StateMemory::dtc_size() as u32;
        }
        unsafe {
            ptr::write(req, result);
        }
    }
    /// # Safety
    /// The host supplies valid request structures and exclusive construction RAM.
    // Only the optional DRAM entry point calls this directly in production;
    // default factories retain this function as their ordinary C ABI entry.
    #[inline(always)]
    pub(crate) unsafe extern "C" fn construct(
        mem: *const NtAlgorithmMemoryPtrs,
        req: *const NtAlgorithmRequirements,
        specs: *const i32,
    ) -> *mut NtAlgorithm {
        // SAFETY: callback pointer arguments are supplied by the host.
        let Some(specs) = (unsafe { Self::specs(specs) }) else {
            return ptr::null_mut();
        };
        let Some(config) = P::configuration(specs) else {
            return ptr::null_mut();
        };
        let Some(layout) = InstanceLayout::new::<P::State, P::Region, P::StateMemory>(config)
        else {
            return ptr::null_mut();
        };
        let (ram, bytes, parameter_count) = unsafe {
            (
                (*mem).sram,
                (*req).sram as usize,
                (*req).num_parameters as usize,
            )
        };
        if parameter_count != config.parameters {
            return ptr::null_mut();
        }
        if ram.is_null()
            || bytes < layout.allocation.size()
            || ram as usize % layout.allocation.align() != 0
        {
            return ptr::null_mut();
        }
        let raw = ram.cast::<Storage<P::State, P::StateMemory>>();
        // SAFETY: checked aligned allocation layout, exclusively available here.
        let (parameters, pages, indices, regions) = unsafe {
            (
                ram.add(layout.parameters).cast::<NtParameter>(),
                ram.add(layout.pages).cast::<NtParameterPage>(),
                ram.add(layout.indices),
                ram.add(layout.regions).cast::<P::Region>(),
            )
        };
        for index in 0..config.regions {
            // SAFETY: each disjoint aligned region slot is initialized in place.
            let mut destination = unsafe { RegionInitializer::new(regions.add(index)) };
            if P::initialize_region(&mut destination, specs, index).is_none()
                || !destination.initialized
            {
                return ptr::null_mut();
            }
        }
        let mut metadata = Metadata {
            // SAFETY: these checked layout regions are disjoint and uninitialized.
            parameters: unsafe { slice::from_raw_parts_mut(parameters.cast(), config.parameters) },
            pages: unsafe { slice::from_raw_parts_mut(pages.cast(), config.pages) },
            indices: unsafe { slice::from_raw_parts_mut(indices.cast(), config.page_indices) },
            initialized: [0; 8],
            initialized_count: 0,
            page_count: 0,
            used: 0,
        };
        if P::metadata(specs, &mut metadata).is_none() {
            return ptr::null_mut();
        }
        let Some(page_count) = metadata.finish() else {
            return ptr::null_mut();
        };
        // SAFETY: checked exclusive construction allocation. The initializer
        // never exposes uninitialized S and success requires every field written.
        let initialized = unsafe {
            P::StateMemory::initialize_at(
                ptr::addr_of_mut!((*raw).state),
                (*mem).dtc,
                (*req).dtc as usize,
                |destination| {
                    let mut writer = RegionInitializer::new(destination);
                    P::initialize_state(&mut writer, specs, super::sample_rate())?;
                    writer.initialized.then_some(())
                },
            )
        };
        if initialized.is_none() {
            return ptr::null_mut();
        }
        // SAFETY: disjoint checked fields; publish only after state and metadata
        // initialization succeed. No whole-Storage value is placed on the stack.
        unsafe {
            ptr::write(
                ptr::addr_of_mut!((*raw).base),
                NtAlgorithm {
                    parameters,
                    parameter_pages: ptr::addr_of!((*raw).pages),
                    v: ptr::null(),
                    v_including_common: ptr::null(),
                },
            );
            ptr::write(
                ptr::addr_of_mut!((*raw).pages),
                NtParameterPages {
                    num_pages: page_count as u32,
                    pages,
                },
            );
            ptr::write(ptr::addr_of_mut!((*raw).parameter_count), config.parameters);
            ptr::write(ptr::addr_of_mut!((*raw).region_count), config.regions);
            ptr::write(ptr::addr_of_mut!((*raw).region_offset), layout.regions);
        }
        raw.cast()
    }
    /// # Safety
    /// base is this registration's live host instance; v/metadata are initialized.
    pub(crate) unsafe extern "C" fn parameter_changed(base: *mut NtAlgorithm, p: i32) {
        let mut context = unsafe { Context::admit(base) };
        P::parameter_changed(&mut context, p);
    }
    /// # Safety
    /// The host supplies a live instance and disjoint contiguous LAST_BUS audio buffers.
    pub(crate) unsafe extern "C" fn step(
        base: *mut NtAlgorithm,
        buses: *mut f32,
        frames_by_4: i32,
    ) {
        let Some(frames) = usize::try_from(frames_by_4)
            .ok()
            .and_then(|n| n.checked_mul(4))
        else {
            return;
        };
        let Some(length) = frames.checked_mul(LAST_BUS) else {
            return;
        };
        if frames == 0
            || buses.is_null()
            || length > isize::MAX as usize / core::mem::size_of::<f32>()
        {
            return;
        }
        // SAFETY: checked representable length; actual allocation is a host contract.
        let (mut context, samples) = unsafe {
            (
                Context::admit(base),
                slice::from_raw_parts_mut(buses, length),
            )
        };
        P::step(&mut context, samples, frames);
    }
    /// # Safety
    /// Active host drawing callback with a live matching instance.
    pub(crate) unsafe extern "C" fn draw(base: *mut NtAlgorithm) -> bool {
        let (mut context, canvas) = unsafe { (Context::admit(base), Canvas::new()) };
        P::draw(&mut context, canvas)
    }
    unsafe extern "C" fn has_custom_ui(base: *mut NtAlgorithm) -> u32 {
        let mut context = unsafe { Context::admit(base) };
        P::has_custom_ui(&mut context) & P::CUSTOM_UI
    }
    /// # Safety
    /// Live matching instance and readable host event.
    pub(crate) unsafe extern "C" fn custom_ui(base: *mut NtAlgorithm, data: *const NtUiData) {
        let (mut context, event) = unsafe { (Context::admit(base), ptr::read(data)) };
        P::custom_ui(&mut context, &event);
    }
    /// # Safety
    /// Live matching instance and writable storage for three pot values.
    pub(crate) unsafe extern "C" fn setup_ui(base: *mut NtAlgorithm, pots: *mut NtFloat3) {
        let mut context = unsafe { Context::admit(base) };
        let values = P::setup_ui(&mut context).map(|value| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.5
            }
        });
        unsafe {
            ptr::write(pots, values);
        }
    }
    /// # Safety
    /// Live matching instance and exclusive host JSON stream for this callback.
    #[inline(always)]
    pub(crate) unsafe extern "C" fn serialise(base: *mut NtAlgorithm, stream: *mut NtJsonStream) {
        let (mut context, mut writer) = unsafe { (Context::admit(base), JsonWriter::new(stream)) };
        P::serialise(&mut context, &mut writer);
    }
    /// # Safety
    /// Live matching instance and exclusive host JSON parser for this callback.
    #[inline(always)]
    pub(crate) unsafe extern "C" fn deserialise(
        base: *mut NtAlgorithm,
        parse: *mut NtJsonParse,
    ) -> bool {
        let (mut context, mut reader) = unsafe { (Context::admit(base), JsonReader::new(parse)) };
        P::deserialise(&mut context, &mut reader)
    }
    /// # Safety
    /// Live matching instance and host prefix buffer of at least the API's minimum size.
    pub(crate) unsafe extern "C" fn prefix(
        base: *mut NtAlgorithm,
        p: i32,
        buffer: *mut c_char,
    ) -> i32 {
        let (mut context, buffer) = unsafe {
            (
                Context::admit(base),
                slice::from_raw_parts_mut(buffer, PARAMETER_UI_PREFIX_SIZE),
            )
        };
        let length = P::prefix(&mut context, p, buffer);
        finish_text(buffer, length)
    }
    /// # Safety
    /// Live matching instance and host string buffer of at least the API's minimum size.
    pub(crate) unsafe extern "C" fn parameter_string(
        base: *mut NtAlgorithm,
        p: i32,
        v: i32,
        buffer: *mut c_char,
    ) -> i32 {
        let (mut context, buffer) = unsafe {
            (
                Context::admit(base),
                slice::from_raw_parts_mut(buffer, PARAMETER_STRING_SIZE),
            )
        };
        let length = P::parameter_string(&mut context, p, v, buffer);
        finish_text(buffer, length)
    }
    unsafe extern "C" fn midi_realtime(base: *mut NtAlgorithm, byte: u8) {
        let mut context = unsafe { Context::admit(base) };
        P::midi_realtime(&mut context, byte);
    }
    unsafe extern "C" fn midi_message(base: *mut NtAlgorithm, status: u8, data1: u8, data2: u8) {
        let mut context = unsafe { Context::admit(base) };
        P::midi_message(&mut context, status, data1, data2);
    }
    /// # Safety
    /// The host supplies count readable bytes for the duration of the callback.
    unsafe extern "C" fn midi_sysex(message: *const u8, count: u32) {
        let count = count as usize;
        if count > isize::MAX as usize || (count != 0 && message.is_null()) {
            return;
        }
        let bytes = if count == 0 {
            &[]
        } else {
            // SAFETY: checked representable nonempty length and host lifetime.
            unsafe { slice::from_raw_parts(message, count) }
        };
        P::midi_sysex(bytes);
    }
    /// # Safety
    /// Live matching instance and writable pointer output. The host consumes
    /// the returned initialized buffer synchronously, without retaining it or
    /// reentering the plugin while accessing its borrowed state.
    unsafe extern "C" fn string_parameter_buffer(
        base: *mut NtAlgorithm,
        p: i32,
        output: *mut *mut c_char,
    ) -> i32 {
        if output.is_null() {
            return 0;
        }
        unsafe { *output = ptr::null_mut() };
        let mut context = unsafe { Context::admit(base) };
        if p < 0 || p as usize >= unsafe { (*context.raw).parameter_count } {
            return 0;
        }
        let Some(buffer) = P::string_parameter_buffer(&mut context, p) else {
            return 0;
        };
        let Ok(length) = i32::try_from(buffer.len()) else {
            return 0;
        };
        let Some(last) = buffer.last_mut() else {
            return 0;
        };
        *last = 0;
        // SAFETY: the trait borrow ties this initialized allocation to the
        // live context; the host's documented non-retention contract applies.
        unsafe { *output = buffer.as_mut_ptr().cast() };
        length
    }
}

/// All export and panic-handler attributes are emitted by the API crate's macro,
/// so a plugin can enforce forbid(unsafe_code), including its registration source.
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        #[no_mangle]
        pub extern "C" fn pluginEntry(selector: u8, data: u32) -> usize {
            $crate::api::plugin::Registration::<
                $plugin,
                { <$plugin as $crate::api::plugin::Plugin>::SPECIFICATIONS.len() },
            >::entry(selector, data)
        }
        #[cfg(not(test))]
        #[panic_handler]
        fn plugin_panic(_: &core::panic::PanicInfo) -> ! {
            loop {}
        }
    };
}
fn finish_text(buffer: &mut [c_char], length: i32) -> i32 {
    if let Ok(index) = usize::try_from(length) {
        if let Some(terminator) = buffer.get_mut(index) {
            *terminator = 0;
            return length;
        }
    }
    if let Some(first) = buffer.first_mut() {
        *first = 0;
    }
    0
}
