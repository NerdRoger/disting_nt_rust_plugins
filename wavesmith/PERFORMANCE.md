# Wavesmith playback profiling — 2026-11-05

Measured on the user's unchanged `scratch` preset: ten sounding CV oscillators,
stereo output, 48 kHz, 24-frame audio callbacks. No notes or musical parameter
changes were sent. Every deployment used the existing Python push script with
`scratch`. Profiling tools were committed separately as `2b99acc`.

## Method

Each run warms up for five seconds, then collects about 100 one-second USB
SysEx reports for 100 seconds. Means are weighted by audio callback counts.
The occasional 101st report is an endpoint effect of asynchronous MIDI arrival;
it is included with its actual callback count. All runs below had zero missing
reports and matching workload metadata.

Timed intervals include oscillator preparation, rendering, gates, per-voice
mixing/panning and energy accumulation. Intervals from all internal chunks are
summed per callback. They exclude pitch-to-frequency setup, waveform rebuilds,
FFTs, the shared level controller, output writes and telemetry transmission.
These are playback-region cycles, not the entire algorithm's CPU percentage.
Interrupts and a small counter-read overhead are included. Keep the same musical
workload when comparing; metadata checks cannot detect every parameter change.

The baseline was repeated after the first candidate (A/B/A). It returned within
0.01%, supporting the observed 1.39% gain. Later incremental changes compare
against the last accepted build, rather than against the previous rejected one.

## Results

| Build / experiment | Reports | Mean cycles per callback | Change vs comparison build | Decision |
|---|---:|---:|---:|---|
| baseline-before | 100 | 37,717 | +0.00% | Baseline |
| baseline-control | 101 | 37,720 | +0.01% | Control |
| specialized-table-runs | 100 | 37,194 | -1.38% | Kept: 91af2a8 |
| inline-pitch | 101 | 33,174 | -10.81% | Kept: 2914b06 |
| unrolled-stereo | 101 | 31,918 | -3.79% | Kept: c19c0d7 |
| six-table-selector | 101 | 31,854 | -0.20% | Discarded: within variation |
| outlined-render-loops | 100 | 31,890 | -0.09% | Discarded: within variation |
| speed-opt3 | 101 | 30,954 | -3.02% | Discarded: code growth |
| speed-opt2 | 101 | 30,694 | -3.84% | Discarded: code growth |
| final-verification | 100 | 31,910 | -15.40% | Selected build, repeated |

The two tiny changes (0.20% and 0.09%) were not retained: they do not establish a
useful benefit relative to per-report variation. The fixed selector also added
special-case code that the existing generic selector does not need.

Compiler overrides were scoped to Wavesmith. Opt-level 2 increased hot code
from 24,728 to 46,134 bytes for a further 3.84% reduction; opt-level 3 used 54,666
hot-code bytes and was slower than opt-level 2. Those overrides were discarded
in favor of retaining code-memory capacity. Opt-level 1 passed optimized native
tests but failed ARM relocation validation, and was never deployed.

The accepted source optimizations increase hot code from the instrumented
baseline's 22,896 bytes to 24,728 bytes, about 1.8 KiB. Cold DRAM code remains
3,460 bytes and waveform/FFT buffers remain 24 KiB. No extra per-voice waveform
buffers, CV quantization, deadband, pitch-smoothing changes or level-controller
changes were introduced by these optimizations.

The final repeated run averaged **31,910 cycles per callback**, compared with
**37,717** for the original instrumented baseline: **15.40% less rendering/mixing
work**. It agrees with the earlier accepted-build measurement within 0.03%.
This is not a claim of a 15.40% reduction in whole-plugin CPU utilization.

The selected build is deployed, telemetry streaming is stopped, and the original
preset remains ringing in stereo CV mode. Further algebraic changes to pitch or
energy calculations, different smoothing behavior, and larger rendering-buffer
redesigns were left out of this round because they need different validation or
trade-offs. The straightforward loop, function-boundary, selector and compiler
experiments above cover the current per-voice hot path without changing its
sample or level behavior.

## Validation and reproduction

Exact-output regression tests cover moving pitches, jitter, table transitions,
waveform-bank changes, fixed and changing pan, mono transitions and individual
energy measurements. Tests retain independent scalar/buffered references.
Full repository validation, optimized native tests where applicable and ARM
relocation validation passed before deployment of each accepted candidate.

```powershell
python wavesmith/tools/monitor_cycles.py --seconds 100 --warmup 5 --label baseline --expect-voices 10 --expect-mode cv --expect-stereo --output target/wavesmith-profiling/new-baseline.csv
```

Add `--compare target/wavesmith-profiling/new-baseline.json` when measuring a
candidate. The stream is opt-in; the monitor disables it in `finally` and closes
both MIDI ports. Use `--stop` after an interrupted process, before deploying.
Do not leave another MIDI client holding the same Windows port.

Detailed CSV/JSON files and experiment binaries are under the ignored
`target/wavesmith-profiling/` directory. They are local artifacts; this summary
is retained in Git. The temporary footer overlay and profiling infrastructure
remain available for future investigations.
