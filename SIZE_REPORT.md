# Rust plugin size optimization — 2026-10-26

The subsequent [structural simplification report](window-comparator/REFACTOR_REPORT.md)
records the latest comparator build at **11,035 allocated bytes**. Measurements
below describe the preceding size-optimization pass.

Built with the project's cached WSL devcontainer (`disting-nt-rust-toolchain:latest`,
image `afd29d73b836`), Cortex-M7 hard-float PIC, Rust plugin profile (`opt-level=s`,
LTO, one codegen unit), and the normal sys-owned relocatable-object packager.
No code was moved to DRAM. No changes were staged or committed.

## Window Comparator results

These are ELF sections marked ALLOC, before any additional firmware allocation
alignment or bookkeeping. They are not measurements of the device's ITC/DRAM
allocation. File size includes symbols, section names and relocation metadata.

| Measurement | C++ baseline | Rust before | Rust final |
| --- | ---: | ---: | ---: |
| Executable section contents | 9,004 | 11,064 | 9,510 |
| Read-only data | 1,519 | 1,479 | 1,479 |
| Writable static data | 796 | 860 | 676 |
| Total allocated section contents | 11,319 | 13,403 | 11,665 |
| Object file size | 21,644 | 36,652 | 35,108 |

All values are bytes. Rust allocated contents fell by **1,738 bytes (13.0%)**.
The gap to C++ fell from **18.4% to 3.1%**. Executable contents are now 506 bytes
larger than C++; combined static data is 160 bytes smaller. Firmware-reported
memory usage and execution time still need device measurement.

## Sequential evaluations

| Change | Allocated bytes after change | Change from previous retained build | Decision |
| --- | ---: | ---: | --- |
| Starting Rust object | 13,403 | — | Baseline |
| Sys memory helpers | 12,215 | −1,188 | Keep |
| API-owned bounded formatted text | 11,908 | −307 | Keep |
| Compact static names and validated definition renaming | 11,741 | −167 | Keep |
| Bulk metadata setter | 11,875 | +134 | Revert |
| Checked parameter groups and block-constant input settings | 11,665 | −76 | Keep |

### Sys: memory helpers

Forward compiler-generated ARM memcpy/memclr helpers to firmware memcpy/memset,
instead of retaining the larger compiler_builtins implementations. Linker wrapping
keeps those C library functions as imports during archive selection. Aligned helper
variants share the general implementation; unused bridges are not selected.

The bridges preserve d0-d7 and FPSCR around the C call, along with the ordinary
C callee-saved register contract. This satisfies the stronger runtime-helper
contract described by [Arm RTABI32](https://github.com/ARM-software/abi-aa/blob/main/rtabi32/rtabi32.rst#memory-clearing-and-setting).
The actual bridge code occupies 52 bytes in Window Comparator. Each bridge uses
72 temporary stack bytes, in addition to the firmware library's own stack usage.
Calls add register-saving overhead; hardware timing was not benchmarked. Native
plugin tests do not execute these bridges, so a separate ARM instruction test was
added and run. Device loading and timing remain the final confirmation.

### API/plugin: formatted text

`text::TextBuffer<N>` owns initialized, terminated text and retains its length.
A small bounded scan when formatting handles interior NULs and malformed reported
lengths. Borrowing its CStr does not rescan the buffer. Window Comparator uses this
for statistic labels, removing the generic 404-byte C-string scanner; supporting
code makes the net saving 307 bytes. Its temporary text object grows by one usize
(four bytes on ARM). No per-instance plugin state was added.

### API/plugin: static names

`metadata::Name` stores an immutable validated static C name as one pointer, with
no public pointer access. `Definition::with_name` preserves validated numeric
metadata while replacing the name. `Metadata::add_named_page` supports page-name
tables. Window Comparator now uses these for its 50 table entries. Table contents
shrink by 200 bytes; other code/data layout changes make the overall saving 167.

### API: bulk metadata experiment

A checked range setter for definition groups increased allocated size by 134
bytes alongside the still-required individual setter. Removed both the new API
and its plugin call sites. Existing initialization tracking and validation remain.

### API/plugin: parameter groups

`ParameterValues::group::<N>` validates a fixed range of values and definitions
once. Audio processing uses local indices for channel, aggregate and counted-gate
groups. Constant local indices allow repeated bounds checks to disappear. Global
input scale/offset are read once per block; per-channel overrides and sequential
audio-bus reads/writes retain their original behavior. No parameter references
are held across reentrant host setters.

## Monosynth

The shared changes also rebuilt `plugins/rusttest.o`. Allocated contents fell from
2,320 to **1,528 bytes** (−792, 34.1%); file size fell from 9,460 to **8,776 bytes**.
Its 16-byte DTC DSP state and framework-owned metadata policy are unchanged.

## Verification

- 63 Rust tests passed: 33 Window Comparator, 2 Monosynth, 3 sys, 20 API, and 5
  compile-fail documentation tests.
- Window Comparator matched the original C++ reference trace, including all
  channel counts, metadata, parameter reentry, audio processing, drawing and presets.
  The C++ reference ran with AddressSanitizer and UndefinedBehaviorSanitizer.
- ABI checks passed: 335 ARM size/alignment/offset/constant comparisons, 51 host
  function signatures, 18 factory callback signatures and the DRAM section macro.
- Both final objects passed supported-relocation, firmware-import, entry-point,
  compiler-metadata and unresolved-Rust-symbol checks.
- All four crates compiled for ARM with Rust warnings denied. Formatting and
  production dependency-layer checks passed; plugins still forbid unsafe code.
- 208 ARM memory-helper cases passed in Unicorn: lengths 0–257, source/destination
  alignments, guard bytes, source preservation, stack alignment/restoration,
  callee-saved core registers, all 32 VFP registers and FPSCR. The simulated C
  library deliberately clobbered its allowed scratch state. Sources were compiled
  for Cortex-M7 and their shared Thumb-2/VFP instructions executed on an ARMv7 VFP
  emulator model; Cortex-M interrupts and lazy FP context handling were not modeled.

Build/test commands, from `rusttest` inside the one-shot devcontainer:

```
cargo fmt --all -- --check
make test
# Optional instruction test; requires Python unicorn.
python3 distingnt-sys/tools/check_memory.py
cargo rustc -p distingnt-sys --target thumbv7em-none-eabihf --release -- -D warnings
cargo rustc -p distingnt-api --target thumbv7em-none-eabihf --release -- -D warnings
cargo rustc -p window-comparator --target thumbv7em-none-eabihf --profile plugin -- -D warnings
cargo rustc -p monosynth --target thumbv7em-none-eabihf --profile plugin -- -D warnings
make all
```

The emulator dependency used for this review is confined to the ignored target
cache; it does not change the devcontainer or add a production/build dependency.
Final device artifacts are `plugins/rust-window-comparator.o` and `plugins/rusttest.o`.
