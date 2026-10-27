# Disting NT Rust plugins

Standalone Rust workspace for Disting NT plugins, copied from the Rust experiment
in `NerdRoger/disting_nt_plugins`. Production plugins depend only on the safe API.

- `distingnt-sys`: faithful C++ header bindings, native ABI bridges and packaging.
- `distingnt-api`: safe Rust registration, host adapters and callback interfaces.
- `window-comparator`: safe Rust Window Comparator plugin.
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

Build outputs are `plugins/rust-window-comparator.o` and `plugins/rusttest.o`
(the monosynth). Cargo intermediates live in ignored `target/`.
The plugins require API-v14-capable firmware.

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

Push the desired version tag to GitHub to build that plugin. Ordinary branch
pushes and pull requests do not trigger builds. Each plugin's workflow also
supports manual runs from the Actions tab.

Both entry workflows call `.github/workflows/rust-plugins.yml`, which checks out
the pinned API submodule, builds the devcontainer with GitHub Actions layer
caching, checks formatting, builds the selected plugin and runs the test suite.
Dockerfile changes invalidate the affected cached layers on the next build.

Tagged builds create a GitHub Release named from the tag (for example,
`window-comparator v0.1.0`) with the selected plugin `.o` attached. Manual runs
on a branch upload an Actions artifact without creating a release. Build
artifacts are retained for 30 days. No custom secrets or container registry
setup are needed; the workflows use the built-in token to create releases.

## Migration

Copied on 2026-10-27 from the current working tree, including uncommitted Rust
work. The Cargo workspace now lives at this repository root. Historical reports
retain references to the original `rusttest` layout. No original source files
were moved or removed. Git was initialized locally with the API registered as a
submodule; no initial root commit or GitHub remote/publication was created.
