# Disting NT Rust plugins

Standalone Rust workspace for Disting NT plugins, copied from the Rust experiment
in `NerdRoger/disting_nt_plugins`. Production plugins depend only on the safe API.

- `distingnt-sys`: faithful C++ header bindings, native ABI bridges and packaging.
- `distingnt-api`: safe Rust registration, host adapters and callback interfaces.
- `window-comparator`: safe Rust Window Comparator plugin.
- `wavesmith`: live segmented oscillator with 2–32 curved sections and an always-visible help footer; see [user guide](wavesmith/README.md) and [developer notes](wavesmith/Developer.md).
- [`drumlab`](drumlab) groups the drum plugins and their shared code.
  - [`kicklab`](drumlab/kicklab/README.md) is a kick drum with 1–8 layers (default 1) and twelve synthesis models.
  - [`snarelab`](drumlab/snarelab/README.md) is a snare drum with 2–5 layers (default 3), four body models, three wire models, and Rim Shot.
  - [`layered-drums`](drumlab/layered-drums) contains the shared playback, scope, and DSP code.
- `monosynth`: Rust port of [Expert Sleepers' C++ monosynth example](https://github.com/expertsleepersltd/distingNT_API/blob/main/examples/monosynth.cpp), created as a Rust learning experiment.
- `distingNT_API`: independent upstream Git submodule, pinned to `1780254aebc3c73b96c351a7b12c325d1f983402`.
- `tools`: ABI, layering and loader validation.
- `.devcontainer`: build environment with Rust and GNU Arm tools.

## Getting started

For a fresh clone, initialize the pinned dependency:

```sh
git submodule update --init --recursive
```

Open this folder in VS Code and reopen in its devcontainer, or build/run the
Dockerfile using wslc without VS Code. From this repository root inside the
build environment:

```sh
make all
make test
cargo fmt --all -- --check
```

Build outputs are `plugins/wavesmith.o`, `plugins/rust-window-comparator.o`,
`plugins/rusttest.o` (the monosynth), `plugins/kicklab.o`, and `plugins/snarelab.o`. Cargo intermediates
live in ignored `target/`. KickLab and SnareLab require API-v15-capable firmware for
background rendering. The other plugins require API-v14-capable firmware.

## Building with wslc from Windows

An existing `disting-nt-rust-toolchain:latest` image contains the same toolchain.
To create an image from this repository's Dockerfile, run from this directory:

```powershell
wslc build -t disting-nt-rust-toolchain:latest -f .devcontainer/Dockerfile .
wslc run --rm --volume "${PWD}:/workspace" --workdir /workspace disting-nt-rust-toolchain:latest make all
wslc run --rm --volume "${PWD}:/workspace" --workdir /workspace disting-nt-rust-toolchain:latest make test
```

## Tests and native code

`make test` runs Rust tests and compile-fail borrowing checks, C++/Rust ABI checks,
ARM placement/loader probes and the C++ comparator reference comparison.
The reference sources are retained under `window-comparator/tests/reference/`
solely to preserve the port's regression tests. The sys crate's C++ shims are
also required to bridge the host ABI. No sibling C++ checkout is needed.

The optional `distingnt-sys/tools/check_memory.py` emulator regression requires
the test-only Python package `unicorn`; normal builds/tests do not require it.

## GitHub Actions

Builds follow the same plugin-specific tag convention as the C++ repository:

| Plugin | Tag pattern | Example |
| --- | --- | --- |
| Window Comparator | `window-comparator/v*` | `window-comparator/v0.1.0` |
| Monosynth | `monosynth/v*` | `monosynth/v0.1.0` |
| Wavesmith | `wavesmith/v*` | `wavesmith/v0.1.0` |
| KickLab | `kicklab/v*` | `kicklab/v0.1.0` |
| SnareLab | `snarelab/v*` | `snarelab/v0.1.0` |

Push the desired version tag to GitHub to build that plugin. Ordinary branch
pushes and pull requests do not trigger builds. Each plugin's workflow also
supports manual runs from the Actions tab.

All plugin workflows call `.github/workflows/rust-plugins.yml`, which checks out
the pinned API submodule, builds the devcontainer with a shared GitHub Container
Registry (GHCR) layer cache, checks formatting, builds the selected plugin and
runs the test suite. All plugins and version tags use the same cache; the first
build populates it, and later builds reuse unchanged layers.
Dockerfile changes invalidate the affected cached layers on the next build.

Tagged builds create a GitHub Release named from the tag (for example,
`window-comparator v0.1.0`) with the selected plugin `.o` attached. Manual runs
on a branch upload an Actions artifact without creating a release. Build
artifacts are retained for 30 days. The workflows use the built-in token with
`contents: write` for releases and `packages: write` for the GHCR cache. No custom
secrets are needed; the cache package is created automatically.

## Migration

Copied on 2026-10-27 from the current working tree, including uncommitted Rust
work. The Cargo workspace now lives at this repository root. Historical reports
retain references to the original `rusttest` layout. No original source files
were moved or removed. Git was initialized locally with the API registered as a
submodule; no initial root commit or GitHub remote/publication was created.

## Required in-place construction

All Rust plugins implement the shared API's `initialize_state` and
`initialize_region` hooks. Structs use `distingnt_api::initialize_region!` to write
fields into the final allocation; scalar/unit states use `set`, and arrays use
`fill`/`fill_with`. There is no value-returning constructor fallback. Keep plugin
code safe and avoid large field-constructor temporaries as well. See
[distingnt-api](distingnt-api/README.md) for the construction contract.


### Rust API adaptation policy

Production plugins must stay compatible with `#![forbid(unsafe_code)]`.
Keep unsafe FFI, pointer admission and ownership enforcement inside
`distingnt-api`/`distingnt-sys`. Every new header capability needs a genuinely
safe plugin-facing path, tested with that restriction. If firmware lifetime or
concurrency guarantees are insufficient, expose the raw sys binding and make
the safe path explicitly unavailable; do not disguise an unsafe requirement as
a safe method or ask individual plugins to relax the rule.
