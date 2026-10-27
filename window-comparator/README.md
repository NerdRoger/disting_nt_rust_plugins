# Rust Window Comparator

Close port of `WindowComparator/windowComparatorAlg.cpp` and `comparatorView.cpp`,
including the active behavior of the Common view, pot, timer, and help helpers.
The Cargo workspace and build commands live at the repository root; the C++ reference fixture lives under this crate's tests/reference directory.

The algorithm is implemented in safe Rust. Native API forwarding, external-data
relocation workarounds, and ARM runtime bridges live in distingnt-sys. The
original C++ algorithm is compiled only for the reference test executable.

## Build and try

From the repository root, in the project's devcontainer:

```
make all
make test
```

The new plugin artifact is `plugins/rust-window-comparator.o` in the repository
root. The synth still builds to `plugins/rusttest.o`.

The Rust comparator has GUID `ATwR` and display name `Rust Window Comparator`,
allowing comparison with the original C++ `ATwc` plugin installed alongside it.
The parameter ordinals, page order, defaults, units, scaling, 1–8-channel
specification, custom UI, and JSON keys match the C++ version. The Rust plugin now
targets API v14 and extends input/output routing to buses 1–64. The reference
harness adapts the C++ algorithm's legacy 28-bus parameter maxima to 64 when
comparing metadata; its processing scenarios remain within the legacy range.
Separate Rust tests exercise input, gates and triggers on buses 60–64.

The production crate enforces `forbid(unsafe_code)`. Only the native simulated
C++ host in `tests.rs` is exempt, in test builds. No plugin production module
admits raw pointers, allocates host regions, or implements C ABI callbacks.

Its only production dependency is distingnt-api, the safe facade over
distingnt-sys. Direct sys access is limited to the native test harness.
Processing uses pointer-free ParameterValues/ParameterInfo views instead of
exposing raw ABI definition slices.

`plugin.rs` implements the API crate's safe `Plugin` trait. It declares the
factory/specification, creates ordinary Rust state and channels, initializes
metadata through the API's validated builder, and implements safe callbacks.
`export_plugin!(WindowComparator)` delegates symbol export, factory registration,
ABI trampolines, and the panic handler to the API crate.

Callbacks use safe API contexts directly. There are no plugin-local host or
painter traits and no context adapter. State/voice references and parameter
setters borrow the same context, so the compiler rejects holding a state
reference across a reentrant setter. There are no runtime borrow locks or
per-callback copies of the full state.

| Safe plugin modules | Responsibility |
| --- | --- |
| `window_comparator.rs` | Crate entry, module declarations, shared callback type and parameter-layout helpers |
| `plugin.rs` | Factory configuration and safe callback entry points |
| `model.rs` | Instance state, validated channel count and audio-driven clock |
| `engine.rs` | Comparator/channel state, aggregation, sequential bus writes and gate/trigger policy |
| `parameter_changes.rs` | Bounds synchronization, range arithmetic and recursion guards |
| `parameters.rs` | Validated definitions, stable ordinals, page construction and parameter text |
| `ui.rs` | Concrete UiState, selection, editing and display orchestration |
| `controls.rs` | Button timing, pot tracking and takeover arithmetic |
| `drawing.rs` | Display geometry, concrete Canvas wrapper, labels and statistic formatting |
| `preset.rs` | Preset schema and JSON behavior |

The API crate owns allocation/layout checks, initialization tracking, static
C-string pointer tables, typed callback contexts, parameter access/notifications,
JSON adapters, Canvas drawing, checked text formatting, and basic audio buffers.
Metadata pointers become visible to C++ only after every definition is initialized
and every page/index is validated. Invalid construction returns null.

All original host-visible behavior remains covered by the C++ reference tests.
Channel reads/writes remain sequential; bounds/center-width recursion ordering is
unchanged; preset errors retain earlier changes.

The plugin enables `SERIALIZATION_IN_DRAM` to select API DRAM entry points for
serialise/deserialise. Its implementation methods are ordinary safe Rust, with
no section annotations. Packaging with `--dram-callees` also moves local callees used
exclusively by DRAM code. This includes the six native JSON forwarding shims in
the current build. Shared ITC callees and address-exposed functions stay put;
audio and drawing callbacks remain in ITC. Calls into
firmware stop the relocation analysis because their implementations are external.

`INITIALIZATION_IN_DRAM` also places requirements calculation, construction and
their exclusive metadata/layout helpers in DRAM. Shared runtime helpers remain
in ITC. Instance state still uses SRAM; this option affects executable code only.

## Verification and fidelity

See [REFACTOR_REPORT.md](REFACTOR_REPORT.md) for the measured simplification steps,
current module structure and final build results.

See [SAFETY.md](SAFETY.md) for audited pointer/borrow lifetimes, reentrant host
call rules, buffer contracts, and the assumptions that remain with the host.
All production plugin code forbids unsafe code; the API crate contains the
necessary unsafe host boundary. UI input is copied before host setters and
parameter text checks formatter lengths before appending its unit suffix.

`make test` compiles the original C++ source against a simulated host with
address/undefined-behavior sanitizers, then compares it against Rust. For every
channel count it compares parameter metadata, page layouts, reentrant parameter
changes, 32 audio blocks cycling through 4, 16, 48, and 128 frames (all buffer
float bits, normalizing NaNs), drawing-command
hashes, and serialized UI flags. Separate regression tests cover bounds updates,
first-block trigger suppression, stacked gates, attenuation overrides, encoder
short/long presses, pot updates, and JSON unknown-member/error handling.

The build checks unresolved symbols and relocations in the final ARM object.
These are host/build checks, not a substitute for loading and testing the plugin
on the Disting hardware.

Source behavior retained for comparison:

- Enter/exit outputs are high for the event block; the recorded trigger countdown
  does not extend them into subsequent blocks in the C++ implementation.
- An aggregate average with zero contributing channels produces IEEE NaN.
- Channels read and write buses sequentially, preserving shared-bus interactions.
- UI time comes from audio blocks; the long-press threshold is 250 ms, pot-release
  blocking is 100 ms, and transient help lasts 90 draw calls.

The transient help buffer is larger than the original 11-byte buffer, to safely
hold formatted values such as `-10.000`; displayed text is unchanged. UI-only
generic C++ helper callbacks that were never connected are not reproduced.

Non-audio code has not been moved to DRAM in this initial port. Build products
stay in Cargo's `target` cache and the final `plugins` output directory;
`make clean` removes them.
