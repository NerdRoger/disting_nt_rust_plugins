# Framework boundary audit

The production Window Comparator crate forbids unsafe code. Its only test-build
exception is the simulated C++ host in `src/tests.rs`. Raw pointers, ABI
trampolines, allocation binding and publication now live in
`distingnt-api/src/api/plugin.rs`, alongside the API's existing host adapters.

The API privately depends on distingnt-sys for the header ABI and C++ shims.
Production plugin dependencies contain only distingnt-api. Raw ABI types appear
only in the simulated-host dev-dependency; safe processing sees ParameterValues
and ParameterInfo, not foreign pointers or raw definition fields.

## Construction and metadata

The framework computes an aligned, nonoverlapping SRAM layout for its C ABI
header, ordinary Rust state, parameter definitions, pages, indices and voices.
It checks the supplied buffer's size/alignment and requires the constructor's
parameter count to match the host's requirements. This also guards against a
safe plugin returning inconsistent configurations between callbacks.

State and voices must implement Copy because the host has no destruction hook;
this prevents silently losing owned Drop resources. State is stored once and
borrowed during callbacks, rather than copied for each callback.

The metadata builder can only be created by the framework. It tracks initialized
definitions with a compact bitset, rejects holes/invalid indices/capacity
overruns, and publishes pointers only after successful completion.
Definitions have a private raw representation. Their checked const constructor
accepts only static CStr names and API-owned enum-table views; it validates
defaults, scaling, units, enum capacities and I/O ranges. Its representation is
the same size as the raw ABI.

Construction failure returns null. Invalid host specifications, short/misaligned
storage and inconsistent parameter counts are rejected before writing.
Failure during user initialization can leave partial bytes in the exclusively
owned construction buffer, but no live instance is published.

## Reentrancy and lifetimes

Contexts are neither Clone nor Copy. A parameter-setter capability exclusively
borrows its context, preventing any reference returned from state/voice access
from remaining active across a reentrant setter or notification. Read-only
parameter handles cannot mutate metadata. Audio and UI contexts expose the
appropriate setter. Compile-fail tests enforce these restrictions.

Audio processing borrows disjoint state, voices, parameter definitions/values and
audio samples. Its context cannot issue setters while those borrows exist.
Metadata updates borrow only local numeric Bounds; pointer-bearing fields cannot
be changed through a safe update. The API writes changes before notifying the
host, retaining nested updates. UI events are copied by the ABI bridge.

Canvas and JSON capabilities carry callback lifetimes. A safe implementation
cannot retain a framework-provided Canvas for a later callback. Text callback
return lengths are checked and terminated by the framework.

## Trusted host obligations

The host must keep registered instances, metadata and callback buffers alive and
correctly sized. It supplies the initialized parameter-value array and writable
contiguous audio buses. Zero-parameter instances may have a null value pointer;
the framework creates empty slices without dereferencing it.

The host must prevent incompatible concurrent access to borrowed instance or
parameter storage, including callbacks from threads/interrupts. Synchronous
reentry through the framework's admitted setters is supported at points where
Rust borrows have ended. Native tests simulate this reentry; they do not verify
firmware interrupt scheduling. The framework adds no IRQ masking or runtime
locking and does not claim to prove the external host's concurrency contract.

Host drawing/JSON/formatting calls must consume borrowed arguments
synchronously and honor their documented buffer contracts. Checks after a
formatter returns cannot prevent a broken formatter from overwriting its buffer.
These are FFI obligations, not properties the Rust compiler can establish.

## Checks

Tests cover poisoned storage, aligned/disjoint layout, initialization failures,
zero-parameter instances, mismatched requirements, incomplete metadata, enum/I/O
validation, text-length errors, shared-bus ordering and reentrant metadata/parameter
updates. The C++ reference compares metadata, UI, audio and preset behavior.
Compile-fail tests cover setter contexts, state-borrow overlap and Canvas escape.
ARM builds check ABI layout, symbols and relocations and reject LLVM bitcode.

The plugin enables SERIALIZATION_IN_DRAM to select API-managed DRAM entry points
for serialise/deserialise, with exclusive callees placed by the packager.
This changes code placement only; the callback lifetime and reentry contracts
remain unchanged. Audio and drawing callbacks retain their existing placement.

The plugin opts into API DRAM initialization entry points for requirements and
construction. All allocation/layout validation and publication rules still
apply. Exclusive helper placement is determined per final object; shared ITC
callers prevent moving a helper. Instance state placement remains SRAM.
