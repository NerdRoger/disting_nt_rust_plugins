# Comparator complexity review and simplification — 2026-10-26

Latest build after DRAM initialization below: **11,035 allocated bytes**.

The plugin now uses safe API contexts directly, a concrete UI state, and a
concrete Canvas wrapper. Production source files fell from **21 to 10**, and
the four plugin-local traits (29 required methods) were removed. The safe API's
Plugin/JSON interfaces remain in use. No unsafe code was added to the plugin.

## Sequential measurements and decisions

All sizes below are allocated ELF section contents, before any firmware
allocation alignment or bookkeeping. They are not device ITC/DTC measurements.

| Step | Allocated bytes | Decision |
| --- | ---: | --- |
| Starting build, after the earlier size optimizations | 11,665 | Baseline |
| Direct context access, initial trial | 11,925 | Revise: excessive inlining |
| Direct context access with shared callback scaling | 11,623 | Keep: simpler structure and 42-byte saving |
| Concrete UiState and UI module naming | 11,623 | Keep: clearer responsibility, no size penalty |
| Concrete Canvas painter, removing the painter trait/adapter module | 11,603 | Keep: simpler drawing and another 20-byte saving |
| Consolidated engine, model and parameter modules | 11,603 | Keep: fewer navigation boundaries, no size penalty |

The net allocated saving in this pass is **62 bytes**. The principal improvement
is source structure, rather than a large runtime-memory reduction. Object file
size fell from **35,108 to 32,340 bytes**; most of that file-size reduction is
symbol/section metadata and must not be interpreted as RAM savings.

The original C++ comparison object has 11,319 allocated bytes. Rust is now
**284 bytes (2.5%) larger**, compared with 18.4% before the earlier size work.
The companion Monosynth still has 1,528 allocated bytes.

## What changed

### Direct API context access

Removed `Parameters`, parameter-change `Host`, UI `Host`, and `Instance`, along
with `context_adapter.rs`. UI and parameter synchronization operate on the API's
typed callback context. Reads copy values and state/voice borrows end before
reentrant setters or metadata notifications. Existing recursion guards and
notification ordering remain intact.

The first version caused LLVM to duplicate scaling at many callback call sites.
Keeping `api::parameters::Parameters::scaled` out of line removed that duplication
without restoring the adapter layer. This is one explicit, measured inlining
decision; there is no blanket inlining policy. DSP processing still uses the
unchanged, inlineable `ParameterValues::scaled`. Hardware timing was not measured.

### Concrete UI and drawing

`UiState` owns selection, edit mode, locking, controls and help visibility. It
replaces the generic-sounding `View` name; there is no view inheritance/framework.
The state field is named `ui`.

Drawing uses a concrete `Painter` around the API Canvas. It supplies text defaults
and statistic formatting, without a trait or a separate HostPainter adapter.
Geometry, display snapshots and formatting are together in `drawing.rs`.
Snapshots remain useful for keeping state borrows short while calling the host.

### Cohesive source modules

| File | Responsibility |
| --- | --- |
| `window_comparator.rs` | Entry point, module declarations, typed callback alias and layout helpers |
| `plugin.rs` | Factory configuration, creation and callback delegation |
| `model.rs` | Instance state, channel-count validation and audio clock |
| `engine.rs` | Comparator/channel state, aggregation, audio block processing and bus writes |
| `parameters.rs` | Definitions, ordinals, pages and parameter text |
| `parameter_changes.rs` | Synchronization, recursion guards and window arithmetic |
| `ui.rs` | Concrete UI state and UI/draw orchestration |
| `drawing.rs` | Canvas wrapper, geometry and displayed text |
| `controls.rs` | Button/pot state machines and takeover arithmetic |
| `preset.rs` | JSON preset schema and partial-failure behavior |

`tests.rs` remains a separate simulated-host test module. Embedded unit tests
moved with their relevant code and were retained. Source lines across production
module files, including their embedded unit tests but excluding `tests.rs`, fell
from 2,315 to approximately 2,158. File and line counts are navigation indicators,
not measures of algorithmic complexity or correctness.

The engine still processes channels sequentially. Input reads are not moved ahead
of earlier channel output writes. Pure comparator/window calculations, checked
parameter groups, validated channel counts, trigger/gate policy and preset
semantics remain explicit. The UI still draws at most four visible channels.
No features were removed to simplify the design.

## Final verification

- All **63 Rust tests** passed: 33 comparator, 2 synth, 3 sys, 20 API, 5 compile-fail
  documentation tests.
- Comparator tests matched the original C++ metadata, audio, UI/drawing, preset
  and reentrant parameter traces for all channel counts. The C++ harness ran with
  AddressSanitizer and UndefinedBehaviorSanitizer.
- ABI verification passed: 335 ARM layout/constant comparisons, 51 host function
  signatures, 18 factory callback signatures and the DRAM section macro.
- API, comparator and synth ARM builds passed with Rust warnings denied.
- Formatting, production dependency separation, final symbol/relocation and
  embedded-compiler-metadata checks passed.

No code moved to DRAM. State fields and SRAM/DTC placement policies are unchanged.
Nothing was staged or committed. Hardware behavior and timing require device
confirmation; the fresh artifact is `plugins/rust-window-comparator.o`.

## Reproduce

From `rusttest` inside the project's one-shot WSL devcontainer:

```
cargo fmt --all -- --check
make test
cargo rustc -p distingnt-api --target thumbv7em-none-eabihf --release -- -D warnings
cargo rustc -p window-comparator --target thumbv7em-none-eabihf --profile plugin -- -D warnings
cargo rustc -p monosynth --target thumbv7em-none-eabihf --profile plugin -- -D warnings
make all
arm-none-eabi-size ../plugins/windowComparator.o ../plugins/rust-window-comparator.o ../plugins/rusttest.o
```

## Follow-up: unused state and audio entry point

Applied three changes together, with one final verification pass:

- Removed the write-only 64-entry trigger countdown array and trigger-length
  field. Event outputs retain the original one-block behavior and stacking order.
- Controls now retain only left/right pot state and the right encoder button.
  Removed middle-pot/left-button bookkeeping and unused pot long-press polling.
  Left encoder rotation, pot press/release blocking, and right-button short/long
  press behavior remain intact.
- Flattened the temporary Block adapter into one fallible audio-processing
  function operating directly on the API's borrowed processing view. Clock and
  first-step state advance only after successful processing.

Allocated section contents fell **11,603 → 11,203 bytes** (400 bytes saved).
The object file fell **32,340 → 31,516 bytes**. The Rust allocated contents are
now 116 bytes smaller than the 11,319-byte C++ comparison object. Instance SRAM
also shrinks from removal of unused state; device allocation figures remain to
be checked. No code was moved to DRAM and no changes were committed.

All 63 Rust tests, the sanitized C++ reference trace, ABI checks, ARM warnings
check, and final loader checks passed. Fresh output: plugins/rust-window-comparator.o.

## Follow-up: parameter synchronization

Separated global-range updates, per-channel range propagation, local parameter
dispatch, bounds synchronization and center/width synchronization into concrete
functions. A shared closure-based guard helper clears recursion flags on both
successful and failed operations without holding state references across host
calls. Range values remain re-read after notifications and between channels.

Added a simulated-host failure test covering both guards, cleanup, and recovery.
All **64 Rust tests**, the C++ comparison, ABI checks, warnings check and loader
checks pass. Allocated contents increased **11,203 → 11,337 bytes** (+134, 1.2%).
This change is retained for clearer control flow and reliable failure cleanup;
it is not a code-size optimization. The parameter-page order lists are unchanged.

## Synchronization size recovery

Compared 21 disposable build variants covering individual/composed function
boundaries, a concrete shared guard dispatcher, and unified global-range updates.
All candidates retained bounds checks and the working copy was changed only
after selecting the smallest implementation.

| Implementation | Allocated bytes |
| --- | ---: |
| Refactored baseline | 11,337 |
| Keep channel dispatcher out of line only | 11,271 |
| Unify global min/max update path only | 11,189 |
| Both changes, retained | **11,135** |

The global-range handler now shares value reading, the opposite-parameter update,
validation/notification, and width recalculation. It still publishes state before
the notification and re-reads the range afterward. A measured inline(never) on
the channel dispatcher keeps its code separate from the global path. DSP code
and the closure-based guard cleanup remain unchanged. No unchecked indexing,
raw pointers or new abstraction layers were introduced.

Recovered **202 bytes**, exceeding the original 134-byte overhead by 68 bytes.
The final allocated contents are 184 bytes smaller than the C++ comparison object.
File size is 32,016 bytes; ELF metadata size is distinct from allocated contents.
All 64 Rust tests, including failed-setter guard cleanup/recovery, the C++ trace,
ABI, ARM warnings and loader checks pass. Hardware timing was not measured.
No code moved to DRAM and no changes were committed.

## Internal layout and UI cleanup

Removed repr(C) from the internal UiState, AudioClock, Channel and Comparator
types. Raw API/ABI structs retain their C layouts. Pot editing now matches
(Pot, EditMode) explicitly to select its parameter and limits together. Named
LONG_PRESS_MS, POT_RELEASE_BLOCK_MS and HELP_DRAW_COUNT replace timing literals;
help_draws_remaining makes the draw-count unit explicit. Page-order lists remain
unchanged as requested.

All 64 tests, the C++ comparison, ABI, ARM warnings and loader checks pass.
Allocated contents fell **11,135 → 11,111 bytes** (24 saved); writable static
data remains 676 bytes. Object file size fell 32,016 → 31,976 bytes. No new unsafe
code, DRAM relocation or commits. Internal layout changes require normal device
confirmation; the firmware-facing ABI is unchanged.

## Sequential API/drawing sharing experiments

Each step was measured against the last retained build in disposable source
copies. Candidates passed the loader check before their size was considered.

| Step | Allocated bytes | Result |
| --- | ---: | --- |
| Baseline | 11,111 | — |
| API non-generic definition validation/publication | 11,041 | Keep: −70 |
| Shared coordinate helper, ordinary optimization | 11,041 | Reject: no reduction |
| Shared coordinate helper, out of line | 11,073 | Reject: +32 |
| Out-of-line drawing text setup | 11,023 | Keep: −18 |

The API retains its generic closure interface, delegates common validation and
publication to a private non-generic method, and preserves notification ordering.
No new inlining attribute was needed there. Making draw_channel, draw_help, shape,
statistic or metadata build out of line increased size, so those experiments were
rejected. Only Painter::text receives a measured inline(never) attribute.

The retained result saves **88 allocated bytes**, with static data unchanged at
676 bytes. Monosynth remains 1,528 allocated bytes. The comparator .o file grew
31,976 → 32,216 bytes because of ELF metadata/padding despite smaller allocated
contents; file size is distinct from device memory consumption.

All 64 tests, sanitized C++ comparisons, ABI, ARM warnings and final loader checks
pass. Both plugins were rebuilt. Sys bindings, safety checks, page-order lists and
DRAM placement are unchanged. No changes were committed; hardware timing remains
unmeasured.

## DRAM placement for preset callbacks

The plugin's serialise/deserialise methods now use the safe API's existing
nt_dram_section macro. ARM symbol inspection confirms both implementations in
`.text.nt_dram`: serialise 76 bytes, deserialise 180 bytes, total **256 bytes**.
The two 20-byte API entry stubs remain in their ordinary code sections, as do
native JSON forwarding shims. Overall allocated contents remain **11,023 bytes**;
this moves code rather than reducing it. No audio/drawing code was relocated.

All 34 comparator tests, including the C++ reference and preset partial-failure
behavior, pass. The ARM warnings and loader checks pass. The fresh artifact is
plugins/rust-window-comparator.o; device loading confirms actual firmware memory
allocation. No changes were committed.

## Follow DRAM-only callees

Walked the actual retained ARM call graph from serialise/deserialise. Rust preset,
UI-state and JSON adapter operations are already inlined into the DRAM roots.
Six remaining native bridge functions are exclusively called from those roots:
rust_nt_json_add_boolean, rust_nt_json_add_member_name, rust_nt_json_boolean,
rust_nt_json_match_name, rust_nt_json_number_of_object_members and
rust_nt_json_skip_member. Each occupies four bytes. They now reside in DRAM,
raising `.text.nt_dram` from **256 to 280 bytes**. Allocated contents remain
**11,023 bytes**. API entry stubs, shared globals/memory helpers, audio and drawing
remain in their existing sections; Monosynth remains 1,528 allocated bytes.

The comparator enables a per-object `--dram-callees` packager option rather than
forcing reusable sys functions into DRAM globally. It follows direct calls and
conservatively retains shared/address-exposed functions, recomputing placement
on every build. Firmware JSON methods are external imports and cannot be moved
by the plugin. Seven placement tests cover transitive/shared/address-exposed and
recursive paths, including real ARM section/symbol/relocation processing. All 64
Rust tests, C++ comparisons, ABI, formatting, ARM warnings and final loader checks
pass. Source API/header bindings are unchanged. No changes were committed.

## DRAM construction and initialization — 2026-10-27

Added a generic opt-in Plugin::INITIALIZATION_IN_DRAM constant, defaulting to
false. Window Comparator enables it; Monosynth remains unchanged. The factory
selects DRAM entry points for requirements calculation and instance construction.
Their private implementation bodies inline into those entries to avoid duplicate
forwarding functions. The existing per-object callee policy moves exclusive
layout, metadata construction, definition naming and sample-rate access code.
Shared memory helpers remain in ITC. Instance state remains in SRAM.

Final `.text.nt_dram` is **2,276 bytes**, up from 280 (1,996 additional bytes).
Overall allocated contents are **11,035 bytes**, up 12. Non-DRAM executable
contents decrease by approximately 1,984 bytes. These are ELF section contents;
firmware alignment/allocation figures need device confirmation.

All 64 Rust tests and 7 placement tests pass, along with the C++ comparison,
ABI, formatting, ARM warnings and loader checks. Generic API constructor tests
exercise the selected DRAM factory path; Monosynth exercises default placement
and remains 1,528 allocated bytes. Raw header bindings and original C++ plugin
sources are unchanged. No changes were committed.

## Consistent serialization placement option

Added generic Plugin::SERIALIZATION_IN_DRAM, defaulting to false alongside
INITIALIZATION_IN_DRAM. With PRESETS enabled, the factory selects either ordinary
or DRAM serialization entry points. Window Comparator enables both placement
options and no longer annotates its methods with nt_dram_section. Shared/exclusive
callee rules remain per-object. No comparator-specific logic was added to the API.

The 40 bytes of serialization ABI entry stubs also move to DRAM. Its section now
contains **2,316 bytes**, up from 2,276; overall allocated contents remain
**11,035 bytes**. Monosynth remains 1,528 bytes with default placement. All 64 Rust
tests, 7 placement tests, C++ comparisons, ABI, ARM warnings and loader checks pass.
No changes were committed.

## Final dead-code audit

Reviewed the sys bindings/native bridges, reusable API adapters and comparator
implementation for obsolete helpers, unused state and warning suppressions.
Native production checks and ARM compilation pass with warnings denied; no
dead-code suppression masks private implementation leftovers. No confirmed
dead production code warranted removal. Unused public header bindings and
reusable adapters are retained intentionally for other plugins.

Final-object inspection confirms the comparator contains only its used globals
and JSON bridges; unused JSON/slot bridges and disabled SysEx/string-buffer
callbacks are absent. Removed a generated Python cache beside the tools and
ignored future rusttest __pycache__ directories. No production behavior changed.

The full 72 Rust tests, 7 placement tests, two ARM callback-placement probes,
C++ reference comparisons, ABI, formatting and loader checks remain passing.
Comparator allocation remains 11,035 bytes (2,316 DRAM code); synth remains
1,528 bytes. Original C++ comparator sources remain unchanged. Nothing committed.

## Optional static initialization hooks

The reusable API now supports factory-wide calculate_static_requirements and
initialise through STATIC_INITIALIZATION (default false). Both use the existing
INITIALIZATION_IN_DRAM choice. The initializer receives exactly the requested
zeroed DRAM as a callback-scoped safe slice, with null/empty handling and an
isize length guard. Persistent shared-memory borrowing is deliberately not
exposed; it requires a separate safe resource adapter.

Neither current plugin enables the hooks. Their factory fields remain None,
and sizes/placement remain unchanged (comparator 11,035 allocated bytes, 2,316
DRAM code bytes; synth 1,528 allocated bytes). New native tests check factory
routing, zeroing, guards and empty/null allocations; a compile-fail test checks
that the memory reference cannot escape. A separate safe plugin ARM probe tests
enabled static hooks in both ITC and DRAM modes and passes loader checks.

## Complete safe factory callback coverage

Audited all 18 callbacks in the current C++ factory. Added opt-in MIDI_SYSEX
and STRING_PARAMETERS hooks, and enabled validated UNIT_STRING metadata. SysEx
receives a bounded byte slice without instance context, matching the C++
factory-wide signature. Editable string buffers borrow initialized plugin byte
storage; the API checks parameter/length bounds, terminates the final byte, and
publishes a pointer only for the host's synchronous, non-retaining access.
Unsupported/empty buffers return zero and null. Host edits need not be UTF-8.
All foreign-pointer handling remains in the API; raw header bindings are unchanged.

Also exposed has_custom_ui as an overridable per-instance hook, restricted to the
declared CUSTOM_UI mask, with the existing fixed-mask behavior as its default.
The API README lists every header callback, Rust hook and enabling option.

Native tests check all 18 factory endpoints with optional capabilities enabled,
their disabled states, SysEx delivery and null/empty handling, string termination,
host edits and unsupported parameters, and dynamic UI masks. A compile-fail test
checks that borrowed string storage cannot escape. The safe ARM probe confirms
SysEx and string-buffer callbacks stay in ITC in both initialization placement
modes, and both probe objects pass loader checks.

All 72 Rust tests (including 7 compile-fail checks), 7 placement tests, C++
comparisons, ABI, formatting, warning-free ARM compilation and plugin loader
checks pass. Rebuilt comparator remains **11,035 allocated bytes**, including
**2,316 DRAM code bytes**; synth remains **1,528 bytes**. Neither existing plugin
enables the new hooks, so there is no code or instance-storage increase.
No changes were committed.
