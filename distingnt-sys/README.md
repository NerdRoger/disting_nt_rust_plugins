# Disting NT raw ABI crate

`distingnt-sys` owns the header-level ABI, native C++ shims, build script and
ARM-object packaging tool. Production plugins use `distingnt-api` instead.

## Header bindings

- `raw`: every core `api.h` type, constant, factory callback and host function;
  `API_VERSION_CURRENT` is 14 and `LAST_BUS` is 64.
- `file`: complete `file.h` declarations, results, flags, handles and file info.
- `wav`: sample folders, sample reads, wavetables, streaming and MIDI folders.
- `microtuning`: Scala scale requests, note union, ratio helpers and callbacks.
- `serialisation`: C ABI forwarding for every public JSON stream/parser method.
- `slot`: C ABI forwarding for every public slot method. Start a slot with
  `let mut slot = NtSlot::EMPTY`, then populate it through `NT_getSlot`.

JSON and slot bridges compile as individual native archive members. Cargo's
`build.rs` owns the archive and watches all upstream headers plus bridge sources.
Only referenced members are linked. Set `DISTINGNT_API_INCLUDE` if the upstream
include directory is not at this repository's default location. Compiler/archive
tools can be overridden by `ARM_NONE_EABI_CXX` / `ARM_NONE_EABI_AR`.

Use `rust_nt_globals()` / `rust_nt_screen()` (or `nt_globals!()`) for external
data access; direct LLVM references to the raw globals can produce relocations
unsupported by the Disting loader. C++ class references are represented as raw
pointers in the forwarding functions. The caller must obey all upstream pointer,
callback, buffer, and lifetime contracts. These functions are intentionally unsafe.

Filesystem calls must execute inside `NT_callFileSystemCode` callbacks. Do not
assume file handles/state persist between callbacks. Asynchronous sample/scale
requests and their buffers must remain alive until the completion callback.

The parameter I/O macros now use `LAST_BUS` as their maximum. For the two
`*_output_with_mode!` macros, provide a string literal without the null byte;
they return a two-element array of parameter definitions. Other parameter macros
accept a pointer to a null-terminated name. Rust's `.len()` replaces ARRAY_SIZE.

`nt_dram_section! { fn helper() { ... } }` places a function in `.text.nt_dram`
and prevents inlining it into ITC. It also accepts impl-method items. No existing
plugin callbacks are moved to DRAM automatically.

Both current plugins now advertise API v14 and require firmware supporting that
API. Plugins use a 96-byte factory on ARM, including the new optional
`string_parameter_buffer` callback. Use `None` for algorithms without editable
string parameters. Earlier numeric parameter ordinals and GUIDs are unchanged.

From the repository root, `make test` runs `tools/check_api.py`: it compiles C++ and Rust
ARM metadata probes and compares every concrete struct/union size, alignment,
public field offset, and constant. Host-function and factory callback signatures
are type-checked against declarations derived from upstream. It also verifies
the DRAM-section macro. This tests ABI compatibility; SD-card operations, WAV
streaming and unused slot/JSON features still require device-side functional tests.

## Packaging

Cargo owns native compilation. Package the Rust static library with:

```
python3 distingnt-sys/tools/link_plugin.py target/thumbv7em-none-eabihf/plugin/libyour_plugin.a plugins/your-plugin.o
```

The tool selects live sections, strips compiler/unwind metadata, and preserves
firmware math imports. It wraps exp2f during archive selection, then restores
its firmware symbol name, preventing compiler_builtins from supplying an
implementation with loader-unsupported absolute Thumb relocations. The packager
also keeps memcpy/memset as firmware imports and forwards ARM memcpy/memclr
helpers through small native bridges. These preserve VFP scratch registers and
FPSCR, as required by RTABI32, in addition to ordinary C callee-saved registers.
Each bridge temporarily uses 72 stack bytes. Firmware still performs the copy
or clear; these bridges do not replace it with a byte-at-a-time implementation.
Unused bridges are discarded. Individual plugins do not need linker workarounds.

Optional `--dram-callees` packaging follows direct calls from explicit DRAM roots
in the retained ARM object, moving only local executable sections whose callers
are all in DRAM. Native bridges are compiled into individual function sections.
Transitive chains and mutually recursive groups are handled; any ITC caller or
non-call/address reference prevents movement. Address-exposed callback functions
are conservatively retained. Inlined helpers already occupy their caller's section.
Undefined firmware imports are not relocated. The decision is per final plugin,
not a global annotation on shared API/sys functions. The packager merges moved
sections into one `.text.nt_dram`, updating symbols and relocations. Policy tests
and an actual ARM object test run as part of `make test`.

An optional instruction-level regression check uses Python's `unicorn` package:

```
python3 distingnt-sys/tools/check_memory.py
```

It compiles the actual Cortex-M7 helper sources and executes their Thumb-2/VFP
instructions on an ARMv7 VFP emulator model. It checks 208 length/alignment cases,
guard bytes, stack alignment, callee-saved core registers, all VFP registers, and
FPSCR against deliberately clobbering simulated C library calls. This does not
model firmware timing or Cortex-M interrupt/lazy-FP behavior; device tests are
still required. Unicorn is a test-only dependency, not a normal build dependency.
