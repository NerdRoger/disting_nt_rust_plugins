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


## Round 2: whole-callback and stage measurements

The second round uses report type 2, committed as `f72c4de`, adding callback-body,
setup, table-build, level-control and output/transition totals to the original
render metric. The timer starts at entry to the Rust audio callback and stops
before telemetry encoding/transmission. It includes normal bookkeeping and
measurement overhead, but not the C ABI wrapper or USB send. Stage totals need
not add to the callback total because chunk clearing and loop overhead are
outside the individual stage timers.

The same ten-voice stereo CV preset, 48 kHz and 24-frame callbacks were used.
The toolchain was rustc 1.99.1 (58cf8f9c3 2026-10-13), with the existing size
optimization/LTO profile. Reports were collected for 100 seconds after warmup;
all completed runs had zero missing messages and matching workload metadata.
Do not directly combine these numbers with round 1: adding instrumentation can
change generated-code layout, and round 1 did not time the entire callback.

| Build | Callback body | Setup | Render/mix | Level control | Callback change vs comparison build | Decision |
|---|---:|---:|---:|---:|---:|---|
| round2-baseline | 47,263 | 7,765 | 32,086 | 4,469 | +0.00% | Baseline |
| round2-prepare-once | 46,346 | 8,498 | 30,455 | 4,460 | -1.94% | Kept: 704bacf |
| round2-controller-loops | 45,775 | 8,498 | 31,000 | 3,315 | -1.23% | Kept: c9f3e12 |
| round2-slew-cache | 45,759 | 8,491 | 30,991 | 3,316 | -0.04% | Discarded: within variation |
| round2-idle-wave | 45,743 | 8,505 | 31,384 | 3,316 | -0.07% | Discarded: within variation |
| round2-exact-pitch-cache | 45,396 | 8,417 | 30,689 | 3,327 | -0.83% | Kept: a0b506b |
| round2-baseline-control | 47,036 | 7,531 | 32,093 | 4,469 | -0.48% | Original build repeated |

The preparation change moves work from render/mix into setup; the stage-only
number would have exaggerated the improvement (5.08% versus 1.94% for the
whole callback). The stereo-controller change saves about 1,144 cycles in level
control, but some render overhead increases; the net improvement is 1.23%.
Its separate left/right power accumulators preserve the original sum order,
and paired gain application preserves each sample's original calculation.

The sample-rate-only slew cache saved 0.04% overall and was discarded. The
unchanged-waveform shortcut saved about 425 build-bookkeeping cycles, but code
layout made rendering more expensive; only 0.07% remained overall, so it too was
discarded, along with its candidate-specific tests. These are examples of why a
local stage improvement is not sufficient evidence of lower total CPU usage.

The retained exact-frequency cache is one callback-local pair of floats. Each
voice's persistent cache is checked first; reuse requires a bit-identical final
octave value. Independent detuning, every representable CV change, phase and
smoothing remain intact. Tests compare the frequency values and oscillator
cache state bit-for-bit with independent conversions, including 0.1-microvolt
changes. Whole-callback improvement was 0.83%; not all of that gain should be
attributed solely to fewer conversions, because render code layout also changes.

A return to the original build measured 47,036 cycles, versus 47,263 initially
(0.48% variation). Almost all of this variation is in setup; original-build
render, build and controller stage means are stable. The final repeated selected
build is recorded below. Gains should be read relative to both baselines rather
than presented with more precision than the experiment supports.

Hot code changed from 24,836 bytes for the instrumented round-2 baseline to
24,812 bytes for the selected build. Cold DRAM code remains 3,460 bytes and
read-only data remains 1,603 bytes. The exact conversion cache adds only a local
8-byte pair; there is no added persistent tuning cache or waveform buffer.

Shared pitch-smoothing trajectories were considered but not implemented here:
independent detuning and voice activation require different state handling and
would alter floating-point recurrence results. Those are a broader DSP redesign,
not another exact-preserving loop optimization. The remaining small output stage
already has a settled-transition fast path. Further arithmetic approximations
or reductions in control-update frequency were also left out of this round.

### Round-2 verification

The final repeated selected build averaged **45,391 callback cycles**, with
100 reports and no gaps. This agrees with its earlier 45,396-cycle run within
0.02%. Compared with the first 47,263-cycle baseline it saves **3.96%**; compared
with the returning 47,036-cycle baseline it saves **3.50%**.
A practical conclusion is **about 3.5–4.0% less callback-body work in this round**,
not a claim of that exact reduction for every preset or block size.

All three successful changes were committed separately. The unchanged-waveform
shortcut and slew cache, including candidate-only tests, were removed. The
selected build is deployed with the same scratch preset, and telemetry streaming
is disabled after collection. The help overlay and monitor remain available.
Final validation passed all 113 Wavesmith tests, the repository checks and tests,
the four Python monitor tests and ARM relocation checks.
