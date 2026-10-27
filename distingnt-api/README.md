# Disting NT safe Rust API

`distingnt-api` is the safe plugin-facing crate. Its private dependency,
`distingnt-sys`, owns raw header bindings, C++ shims and native build tooling.
Production plugins declare only the API dependency:

```toml
[dependencies]
distingnt-api = { path = "../distingnt-api" }
```

There are no public raw modules, host FFI functions or pointer-bearing ABI
struct re-exports. Scalar constants, GUID helpers and pointer-free UI event
values remain available at the crate root. Raw clients explicitly depend on
`distingnt-sys` instead.

## Framework and adapters

Implement `api::plugin::Plugin` with ordinary State/Voice values, a StateMemory
policy, factory specifications, validated metadata and safe callbacks. Invoke
`export_plugin!(YourPlugin)` in a no_std crate enforcing forbid(unsafe_code).
See Window Comparator's plugin.rs and Monosynth's lib.rs for complete examples.

Set `Plugin::INITIALIZATION_IN_DRAM = true` to select DRAM entry points for
requirements calculation and construction. It defaults to false for other
plugins. Their implementation bodies inline into the selected entry points;
packaging with `--dram-callees` also moves exclusive construction helpers while
retaining shared runtime helpers in ITC. This places code, not instance state.

`Plugin::SERIALIZATION_IN_DRAM` works the same way for serialise/deserialise
when `PRESETS` is enabled. Both placement options default to false, are selected
independently per plugin, and keep section annotations inside the API. Plugin
callback implementations remain ordinary safe Rust methods.

Enable `STATIC_INITIALIZATION` to supply the optional factory-wide
`calculate_static_requirements() -> StaticRequirements` and
`initialise(StaticMemory<'_>)` hooks. Both follow `INITIALIZATION_IN_DRAM`,
including their exclusive callees. Requirements are measured in DRAM bytes.
The initializer receives a zeroed, writable slice covering exactly that
allocation; zero bytes permit a null host allocation. These hooks run during
scan/loading, separately from per-instance construction. Disabled hooks remain
None and compile away, as in the existing comparator and synth.

The initialization slice is callback-scoped and cannot be stored as a permanent
reference. Persistent shared-DRAM/table access from later callbacks still needs
a dedicated safe resource adapter; these hooks alone do not provide one.

Every one of the current C++ factory's 18 callback endpoints is available through
`Plugin`. Required framework callbacks are registered automatically; optional
ones are enabled as follows:

| C++ callbacks | Rust hooks | Enable with |
| --- | --- | --- |
| calculateStaticRequirements, initialise | calculate_static_requirements, initialise | STATIC_INITIALIZATION |
| calculateRequirements, construct | configuration, create, create_voice, metadata | Always supplied by the framework |
| parameterChanged, step, draw | parameter_changed, step, draw | Always supplied |
| midiRealtime, midiMessage | midi_realtime, midi_message | MIDI |
| hasCustomUi, customUi, setupUi | has_custom_ui, custom_ui, setup_ui | Nonzero CUSTOM_UI mask |
| serialise, deserialise | serialise, deserialise | PRESETS |
| midiSysEx | midi_sysex | MIDI_SYSEX |
| parameterUiPrefix, parameterString | prefix, parameter_string | PARAMETER_TEXT |
| stringParameterBuffer | string_parameter_buffer | STRING_PARAMETERS |

`has_custom_ui` can inspect instance state to vary the overridden controls;
its result is limited to the declared `CUSTOM_UI` mask. Its default returns
the entire mask, preserving existing plugins' behavior.

`midi_sysex(&[u8])` receives the exact message bytes. This callback is
factory-wide: the C++ signature supplies no instance pointer. It is independent
of the per-instance channel/realtime `MIDI` option.

For editable text, use `UNIT_STRING` metadata and implement
`string_parameter_buffer` to return `Some(&mut context.state_mut().buffer)`
for the requested parameter, with a Copy byte-array field such as `[u8; 32]`.
The returned length includes space for NUL. The API terminates the last byte;
the host can read/edit the initialized bytes synchronously and does not retain
the pointer. Empty/unsupported buffers return zero and null. Byte storage avoids
assuming host edits produce valid UTF-8. The borrow prevents setters or other
state access while the buffer is borrowed. The host must not reenter the plugin
or access the buffer concurrently while consuming the returned pointer.

SysEx and string-buffer callbacks remain in ITC, including when initialization
or serialization is selected for DRAM. Disabled callbacks remain null and their
implementations compile away; no new storage is added to existing instances.

- `plugin::{Context, Configuration, Metadata}` handles registration, checked
  allocation, initialization and publication through internal ABI callbacks.
- `parameters::{Parameters, ParameterValues, ParameterInfo, Bounds}` exposes
  checked values, borrowed CStr names, scalar metadata and numeric updates.
  Raw definitions and foreign pointers remain private.
- `drawing::{Canvas, Shape, Alignment, TextSize}` and JSON reader/writer adapters
  are provided by callback contexts; raw constructors are crate-internal.
- `audio::AudioBuses`, checked text formatting, `math::exp2`, enum string tables,
  and sample-rate access provide reusable, allocation-free host operations.
- `text::TextBuffer<N>` owns bounded formatted text and exposes a borrowed CStr
  without rescanning on every draw. It handles malformed reported lengths.
- `metadata::Name` stores static C names compactly; `Definition::with_name` and
  `Metadata::add_named_page` accept these without exposing pointers to plugins.
- `ParameterValues::group::<N>` validates a fixed parameter range once; local
  constant indices can then compile without repeated full-instance checks.
- Sealed `state_memory::{Sram, Dtc}` policies select storage statically.
  Window Comparator retains SRAM state; Monosynth retains its 16-byte DTC DSP
  state. Monosynth now uses framework-owned per-instance metadata in SRAM.

State/voices must be Copy because the host has no destruction callback. This
constraint does not cause whole-state copies on each callback. Rust borrowing
prevents state references from spanning reentrant setters. Small wrappers use
ordinary inline hints; there is no allocation or dynamic dispatch.

The framework covers numeric/enum/I/O metadata, audio, parameter changes,
UI/drawing, presets, text and editable-string buffers, MIDI channel/realtime and
factory-wide SysEx messages, and value state in SRAM/DTC, plus bounded
factory-wide static initialization. Heap-owned Drop state, persistent
shared-memory access, other memory regions and asynchronous APIs require
further framework support.
The raw sys crate remains available for advanced clients.

The host must honor allocation/buffer lifetimes and prevent incompatible
concurrent callbacks. No IRQ masking is added, and firmware scheduling is not
proved by native tests. See Window Comparator's SAFETY.md for the contract.

## Tests and builds

`make test` checks the sys ABI against C++, validates dependency separation,
runs framework/adapter/plugin tests and C++ reference comparisons, and runs
compile-fail checks for facade privacy and callback borrowing.

Only simulated-host tests have a direct sys dev-dependency. The API's
`host-test` feature enables an unsafe admission hook for those native harnesses;
production dependencies do not enable it.

`make all` uses Cargo static libraries and the sys-owned object packager, then
checks loader symbols/relocations. Plugins do not compile/link native shims
themselves. Code placement is unchanged; no functions move to DRAM implicitly.
