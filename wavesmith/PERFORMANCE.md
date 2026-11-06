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


## Round 3: code-space versus CPU review

This review deliberately tested removing optimizations from the combined build,
instead of assuming their original incremental gains were still independent.
The workload and protocol were the same: ten sounding CV voices, stereo output,
48 kHz, 24-frame callbacks, 100-second measurements after five seconds of warmup.
All reported runs completed with zero missing messages. Code sizes below are
ITC-bound executable sections, excluding cold DRAM code and read-only data.

| Build / removal candidate | ITC code | Callback cycles | CPU change vs its comparison build | Decision |
|---|---:|---:|---:|---|
| round3-current | 24,812 B | 45,578 | +0.00% | Starting build |
| round3-no-specialization | 23,608 B | 45,967 | +0.85% | Kept: 9312d4d |
| round3-no-unrolling | 23,304 B | 47,678 | +3.72% | Rejected |
| round3-compact-stereo | 23,344 B | 46,963 | +2.17% | Rejected |
| round3-two-sample-stereo | 23,472 B | 45,293 | -1.47% | Kept: 1735adc |
| round3-no-pitch-sharing | 23,392 B | 45,414 | +0.27% | Kept: 8da7ca9 |

The specialization removal compares with the starting build. Stereo alternatives
compare with the accepted no-specialization build. Pitch-sharing removal compares
with the selected two-sample mixer. Positive cycle changes mean slower execution.
Do not add these percentages or compare alternatives against different baselines.

### Decisions

- Removed the specialized moving-pitch table/lane loops: **1,204 bytes recovered**
  for **0.85% more callback cycles**. This is a better space/time balance than
  retaining four specialized loop variants. Pitch-helper inlining remains.
- Rejected complete stereo-unrolling removal: only 304 further bytes saved for
  3.72% more callback cycles. The compact fixed-pan scalar loop was also rejected:
  264 bytes saved for 2.17% more callback cycles.
- Reduced stereo unrolling from four samples to two instead: **136 bytes saved**
  and **1.47% fewer callback cycles** in this context. This is both smaller and
  faster in the measured combined build, with exact sample/energy agreement.
- Removed the small shared-pitch cache: **80 bytes saved**, a small 0.27% measured
  callback penalty, and simpler conversion code. This is a judgment call favoring
  space and simplicity for a marginal gain; it is not a zero-cost removal.
  Each voice's original persistent exact cache remains, with no CV deadband.
- Kept the original moving-pitch fast path and explicit pitch-helper inlining:
  their measured benefits were strong for their code cost. Kept once-per-callback
  preparation and shared stereo level calculations, which saved code and cycles.

Final ITC code is **23,392 bytes**, versus **24,812 bytes** at the start of this
review: **1,420 bytes recovered (5.72%)**. Relative to today's original 21,170-byte
build, net growth is now 2,222 bytes (10.50%), instead of 3,642 bytes (17.20%).
Cold DRAM code remains 3,460 bytes; read-only data remains 1,603 bytes. Waveform
and FFT buffers are unchanged. ELF file size includes symbols and relocations
and must not be read as the ITC allocation.

The byte deltas differ from the original historical additions because inlining,
function layout and other retained optimizations interact. These removal tests
measure the actual current trade-offs. Small timing differences can also vary
with reloads and CV/cache activity, so the final combined build is repeated
below rather than claiming precise CPU gains from every byte recovered.

Profiling infrastructure remains intentionally present for these comparisons;
the SysEx stream is opt-in and is stopped by the monitor. This is not a build
with all diagnostic code stripped. Candidate-specific code/tests from rejected
alternatives are absent from the final source.

### Round-3 final verification

The selected build repeated at **45,415 callback cycles**,
versus **45,578** at the start of this review (-0.36%).
This is within the previously observed reload/setup variation, so the practical
conclusion is **5.72% less ITC code with essentially unchanged CPU performance**,
not a claim of a meaningful CPU speedup. The repeat completed with 101
reports and no missing telemetry, and agrees with the earlier selected-build
run (45,414 cycles) within 0.02%.

All accepted removals/changes were committed separately. The final source was
explicitly rebuilt and passed all 112 Wavesmith tests, the repository test/check
suite, four Python monitor tests and ARM relocation validation. The chosen build
is deployed with `scratch` unchanged; telemetry is stopped and MIDI ports are
closed after collection. Profiling itself remains available for future tests.


## Round 4: two small arithmetic experiments

Both candidates were tested independently against the same unmodified build,
with the existing ten-voice stereo CV preset, 48 kHz and 24-frame callbacks.
Each run used 100 seconds after five seconds of warmup. A return to the original
build checked temporal variation before selecting either arithmetic change.

| Build | ITC code | Callback-body cycles | Change vs initial baseline | Decision |
|---|---:|---:|---:|---|
| round4-current | 23,392 B | 46,113 | +0.00% | Baseline |
| round4-gain | 23,324 B | 45,085 | -2.23% | Gain candidate |
| round4-reciprocal | 23,400 B | 45,835 | -0.60% | Discarded |
| round4-control | 23,392 B | 45,673 | -0.95% | Original repeated |

Precomputed gain changes `(sample * level) * 5` to `sample * (level * 5)` only
while level is constant and the open-gate renderer can run. It removes a multiply
from each sample and does not add persistent memory. The 68-byte reduction is
measured from the complete ARM object, not estimated from source lines.

That reassociation is not bit-identical. Tests compare the fast renderer with
the unmodified scalar multiplication order at all level settings from -60 to
0 dB, three sample rates and four pitches, covering moving and settled pitch.
Maximum observed per-voice sample error was 0.00000047683716 V (0.48 microvolts).
The bound is four f32 epsilon units times max(1, absolute expected sample);
pitch/phase/oscillator state comparisons remain exact. Existing playback tests
use this explicit sample bound rather than falsely claiming bit identity.

The reciprocal experiment cached `1/sample_rate` and used multiplication for
pitch increments. It passed numerical tests (maximum relative increment error
0.00000011917944, about 0.00021 cents), and independently added eight ITC bytes
and forty state bytes across ten voices. But its 0.60% apparent CPU saving was
smaller than the 0.95% shift in the returning original build. Against that
returning control it was actually slower. No reliable end-to-end gain was
established, so the reciprocal field, arithmetic and candidate-only test were
discarded. Original division-based pitch calculation remains.

A combined candidate was built and numerically validated offline (23,416 ITC
bytes), but was not deployed or benchmarked: the reciprocal did not justify
retention on its own. No performance claim is made for that unused combination.

The selected gain-only build is repeated below against the returning control.
The measured region remains the callback body before telemetry serialization,
not the firmware's entire displayed CPU load. All reports had zero missing
messages and identical workload metadata. Detailed CSV/JSON files and temporary
candidate artifacts remain in ignored `target/wavesmith-profiling/`.

### Round-4 selection and repeat

Gain-only repeated at **44,853 callback cycles**, **1.80%
below the returning original build**. The first paired comparison was 2.23%
lower. A reasonable description is **about 1.8–2.2% fewer measured callback
cycles in this workload**, with **68 fewer ITC bytes** and no additional
persistent state. Render-only means for the two gain runs agreed within 0.003%;
setup and original-build timings still exhibit reload/CV variability.

Final ITC code is **23,324 bytes**, down from 23,392. The reciprocal and its test
are absent; the original division-based pitch math remains. The gain-only source
passed all 113 Wavesmith tests in debug and optimized builds, repository checks,
four monitor tests and ARM relocation validation. It is deployed with the same
scratch preset. The repeated run had 101 reports and no gaps, and the
monitor stopped streaming and closed MIDI ports on exit.


## Round 5: fixed-point per-sample pitch tracking

The existing ten-voice stereo CV preset was measured at 48 kHz with 24-frame
callbacks. Four alternating runs used 100 seconds after five seconds of warmup;
each received 100 or 101 reports, with no missing messages or workload changes.
No MIDI notes or musical parameter changes were sent.

| Build | Callback-body cycles | Rendering cycles | Callback change vs initial baseline |
|---|---:|---:|---:|
| round5-current | 44,838 | 30,329 | +0.00% |
| round5-fixed-pitch | 39,265 | 24,642 | -12.43% |
| round5-control | 44,824 | 30,332 | -0.03% |
| round5-fixed-repeat | 39,272 | 24,649 | -12.42% |

The repeated original differs from the initial baseline by only -0.03% in whole
callback cycles. The fixed-point repeat is **12.39% below that returning control**
in callback-body cycles and **18.74% below it** in rendering cycles. The two
fixed-point callback means agree within 0.02%. These are measured cycle savings
for this CV workload, not percentage points of the firmware CPU meter or a
claim that MIDI and CV now have equal cost. Frequency conversion still uses the
original floating-point exponent/division and exact per-voice cache.

Current and target pitch increments now use the existing 32-bit phase-step
units. An integer Q30 coefficient applies the approximately 1 ms exponential
slew, rounded each sample; a minimum one-unit move prevents sticking short of
the target. At 48 kHz that unit is about 0.000011 Hz. The anti-alias cache uses
integer lower/upper thresholds equivalent to its existing 1 mV selection band;
conversion back to floating point happens only when selecting new tables.
Pitch-target changes are not suppressed with a CV-input deadband. The slew
coefficients are cached until sample rate changes.

This deliberately changes rounding relative to the old f32 pitch recurrence.
Numerical tests compare against a double-precision exponential reference with
an explicit accumulated rounding bound, verify monotonic tracking without
overshoot, exact eventual settling, and less than 0.002 cents steady tuning
error at the tested 20 Hz through 12 kHz frequencies. Tests cover four sample
rates (32/44.1/48/96 kHz), sub-millivolt ramps in both directions, live rate
changes, MIDI retuning and the existing anti-alias selection/sweep checks.
The old scalar waveform-reference test aligns phase for its waveform/gain
comparison and separately bounds pitch error; fast and general integer
renderers retain exact pitch/phase/state comparisons. This does not claim
bit-identical audio to the old floating-point trajectory.

ARM output uses native signed multiply-accumulate-long instructions for the
pitch recurrence, with no 64-bit arithmetic runtime helpers. ITC code increases
from **23,324 to 23,468 bytes (+144 B, +0.62%)**. Cold code remains 3,460 bytes
and read-only data 1,603 bytes. Persistent oscillator state increases by four
bytes per voice, **40 bytes across ten voices**; no additional waveform buffers,
FFT work or dynamic allocations are introduced.

The change was retained for its repeatable cycle saving and small code cost.
All 115 Wavesmith tests pass in debug and optimized host builds, repository
checks and ARM relocation validation pass, and the final object was explicitly
rebuilt. The fixed-point version is deployed through the supplied script with
scratch; the monitor stopped SysEx streaming and closed its ports on exit.
CSV/JSON measurements and comparison binaries remain under ignored
`target/wavesmith-profiling/`. A listening check on smooth CV glides remains
useful because numerical agreement cannot establish subjective equivalence.
