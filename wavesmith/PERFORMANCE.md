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


## Round 6: lookup conversion and shared CV factors

Two candidates were built independently against the fixed-point-pitch build.
The existing ten-voice stereo CV preset, 48 kHz and 24-frame callbacks remained
unchanged. Every run collected 100 seconds after five seconds of warmup, with
100 or 101 reports, identical workload metadata and no missed messages.
Only the supplied deployment script and scratch preset were used. No MIDI
notes or musical parameter changes were sent.

| Build | Callback-body cycles | Setup cycles | Rendering cycles |
|---|---:|---:|---:|
| round6-current | 39,516 | 8,601 | 24,668 |
| round6-lookup | 39,168 | 8,255 | 24,666 |
| round6-control | 39,515 | 8,606 | 24,662 |
| round6-factored | 38,290 | 8,246 | 23,779 |
| round6-lookup-repeat | 39,166 | 8,255 | 24,664 |
| round6-combined | 38,050 | 8,007 | 23,778 |
| round6-factored-repeat | 38,062 | 8,018 | 23,780 |
| round6-combined-repeat | 38,047 | 8,007 | 23,774 |

### Lookup-only experiment: discarded after interaction testing

A 129-entry fractional-octave table was generated at runtime via
calculateStaticRequirements/initialise, using 516 bytes of firmware-owned DRAM
shared by the whole Wavesmith factory. It was not stored per instance or embedded
in read-only data. The static API exposes factory DRAM, not DTC. A checked,
immutable shared-table API handle was prototyped to keep raw pointers outside
plugin code; initialization and memory-admission tests passed.

Linear interpolation, integer octave scaling and the existing per-voice exact
cache replaced frequency-conversion exp2 calls. A dense sweep from -16 through
+16 octaves measured maximum tuning error **0.0065601 cents**, checked
monotonicity and exact octave points. Host integration tests used an explicit
0.01-cent tuning budget instead of falsely requiring the old f32 rounding.
The independent candidate passed all 116 Wavesmith tests and repository checks.

Lookup-only used **23,576 ITC bytes (+108)**, **3,588 cold-code bytes (+128)**,
unchanged 1,603-byte rodata and 516 factory DRAM bytes, plus a four-byte global
handle. It repeated a **0.88% whole-callback saving** versus the returning
original. However, its added value overlapped with CV sharing: the first
combined comparison suggested another 0.63%, while the returning sharing-only
build was faster than its initial run. Against that returning build the repeated
combination saved only **0.04%**, within ordinary timing variation.

The combined build cost 23,976 ITC bytes (+140 over sharing alone), 3,628 cold
code bytes (+128), and the shared DRAM allocation. Its fixed voice tuning
factors used the original exp2 so two lookup approximation errors would not
compound. It passed 118 debug and optimized host tests. Those numerical tests
do not establish an incremental CPU benefit, so the lookup, static callbacks,
shared-table API, API accessor change, probe adjustment and candidate-only tests
were all removed from final source. No factory allocation is retained.

### Shared CV factors: selected

CV frequency is factored into `440 * 2^tuning * 2^CV`. Root/transpose/fine
conversion is cached per voice; input voltage conversion is cached per bus and
shared even when voices have different fine tuning or transpose. Cache entries
retain exact voltages across callbacks; no CV deadband or smoothing change is
introduced. Zero volts has the exact unity factor. Each callback can use up to
one distinct bus per voice, and rerouting validates both the bus and voltage.
MIDI keeps its existing exact combined-pitch cache. Final frequencies retain
the original limits, now applied after multiplication.

This reassociation changes f32 rounding slightly. Numerical tests cover detuned
voices, positive/negative inputs up to 10 V, frequency clamps, shared and separate
buses, routing changes and input changes of one microvolt. Maximum measured
tuning difference in the numerical test is **0.000643 cents**, below its
0.002-cent budget. An integration test proves ten detuned voices share a single
conversion per changing CV input and verify each actual prepared frequency.
Unchanged signals require no repeated conversions across callbacks.

Sharing alone costs **23,836 ITC bytes (+368)**, **3,500 cold-code bytes (+40)**
and unchanged rodata. The cache itself is 124 bytes on ARM; compiler layout
reports State increasing from **2,128 to 2,248 bytes (+120)**, and its storage
wrapper from 2,168 to 2,288 bytes, because four bytes of padding are reused.
No DTC, factory DRAM, additional waveform buffers or dynamic allocations are
added. The source uses the voice-count constant for cache capacity.

Sharing-only saved **3.10%** in its first callback comparison and **3.68%** on
repeat versus the returning baseline. Its render means agree within 0.003%,
but setup timing varied by about 228 cycles between loads. The marginal lookup
claim is therefore rejected rather than interpreting that shift as an added
lookup benefit. A final original/selected pair follows below to check the
retained improvement again. These percentages are callback-body cycle changes,
not percentage points of the firmware's total CPU meter.


### Round-6 final verification

| Build | Callback-body cycles | Setup cycles | Rendering cycles |
|---|---:|---:|---:|
| round6-final-control | 39,283 | 8,376 | 24,661 |
| round6-final-selected | 38,060 | 8,018 | 23,777 |

The final sharing-only pair saves **3.11% of callback-body cycles**, **4.27% of
setup cycles** and **3.58% of rendering cycles**. The final selected mean agrees
with the prior sharing-only repeat within 0.01%. The returning original itself
is 0.59% faster than the first two original runs, primarily in setup, reinforcing
why the marginal incremental lookup saving was not retained. A defensible
result is about **3.1% additional end-to-end callback savings** in this workload,
with the lookup experiment discarded. Improvements depend on the configured
input sharing; independent-input timing was not benchmarked on the device.

The final object is byte-identical to the independently tested sharing-only
object, passes ARM relocation validation and is deployed using scratch.
All 117 Wavesmith tests pass in debug and release builds; repository checks,
other plugin tests, API tests/doctests and monitor tests pass. API/sys changes
and candidate-only lookup tests are absent from the final diff. No telemetry
protocol or musical parameter changes were needed. Both final runs received
101 reports with no missed messages; streaming was disabled and ports closed
when the monitor exited. Raw measurements and rejected alternatives remain
only in ignored target/wavesmith-profiling for audit.


## Round 7: anti-alias selection bounds proof

Each 100-second run (five seconds warmup) used the unchanged ten-voice stereo
CV preset at 48 kHz and 24 frames. All reports matched workload metadata and
arrived without gaps. Only diagnostic commands were sent.

| Build | Callback-body cycles | Setup cycles | Render/mix cycles |
|---|---:|---:|---:|
| round7-base | 38,054 | 8,018 | 23,771 |
| round7-bounds | 36,302 | 8,116 | 21,953 |
| round7-control | 38,063 | 8,018 | 23,780 |
| round7-chunk-bounds | 36,213 | 8,258 | 21,710 |
| round7-bounds-repeat | 36,558 | 8,327 | 21,997 |

The original repeated within 0.03%. The selected strict current/target proof
saved **4.60%** initially and **3.95%** on repeat versus the returning original.
Render/mix saved about 7.5-7.7%; setup timing varied between loads. A useful
claim is about **4% less whole-callback work in this workload**, not the larger
render-only percentage or CPU-meter percentage points. The shortcut's eligibility
varies near selection boundaries; fallback work remains necessary there.

When both current and target phase steps are strictly within cached selection
bounds, the monotonic integer slew cannot cross those bounds in the chunk.
The fixed-selection renderer tracks exactly the original per-sample phase steps
while omitting repeated range checks and table-binding decisions. Steady pitch
retains its separate specialization. Gate/level transitions, waveform fades and
possible table crossings retain the existing general paths. There is no CV
quantization, smoothing approximation, new table, or extra persistent state.

The selected object uses **24,440 ITC bytes**, up **604 bytes** from 23,836.
Cold code and rodata remain 3,500 and 1,603 bytes. This trade-off was accepted
for the repeated roughly 4% callback reduction. A tighter chunk-travel proof
was also tested: its 24,512 ITC bytes cost another 72 bytes, while its 4.86%
whole-callback saving did not establish a reliable incremental gain against the
variable simpler-path measurements. That refinement and its dedicated test
were removed, retaining the smaller proof.

118 Wavesmith tests pass in debug and release, with repository/ABI checks and
ARM relocations validated. Threshold-edge tests check strict bounds, and fast
versus scalar paths retain exact oscillator/pitch/phase-state comparisons.
Existing gain-regrouping sample-error bounds remain unchanged. Sweep, jitter,
MIDI retune and waveform-publication tests cover fallback transitions. This is
an exact integer-pitch optimization and does not require a listening decision.
Raw data and rejected refinement sources stay in ignored target profiling files.


## Round 8: direct CV phase targets

This candidate was tested independently on top of the committed bounds proof.
The same unchanged ten-voice stereo CV preset, 48 kHz, 24-frame callback and
100-second/five-second-warmup measurements were used. Reports had no gaps or
workload metadata changes.

| Build | Callback-body cycles | Setup cycles | Render/mix cycles |
|---|---:|---:|---:|
| round7-bounds-repeat | 36,558 | 8,327 | 21,997 |
| round7-direct | 36,238 | 8,041 | 21,966 |
| round7-direct-control | 36,579 | 8,334 | 22,011 |
| round7-direct-repeat | 36,266 | 8,079 | 21,955 |

The returning bounds-only control agreed within 0.06% with its earlier run.
Direct targets saved **0.87% initially** against the preceding bounds-only
run and **0.86% on repeat** against the returning control. Setup saved roughly
255-293 cycles against that returning control. Rendering has substantial
variation as the strict bounds shortcut changes eligibility, so the modest
whole-callback result is reported rather than attributing the whole reduction
to arithmetic removal. The final direct-target mean is **4.72% below** the
original returning pre-bounds control (38,063 cycles).

CV-mode root/transpose/fine frequency is normalized once into phase-step units
when tuning or sample rate changes. Each changing CV factor then produces the
phase target with multiplication and limits, avoiding the intermediate Hertz
calculation and division. Stable input factors reuse the target across callbacks.
MIDI retains the existing conversion/target arithmetic; its general prepare
invalidates the CV coefficient cache. No input deadband, quantization or pitch
smoothing change is introduced.

The reassociated arithmetic is not bit-identical. Tests cover 32/44.1/48/96 kHz,
positive/negative input sweeps through frequency limits, tuning changes, path
changes and tiny factor changes. Full-range comparisons use a phase-resolution
aware bound; above 20 Hz maximum observed difference is **0.001870 cents**, below
the 0.002-cent budget. At sub-audio rates a single existing phase LSB is relatively
large in cents, so those cases use the absolute phase bound instead. Host tests
now observe the actual target phase-derived frequency, explicitly including
existing pitch limits and one integer phase LSB in their tolerance. The old
Hertz calculation remains only as a test reference.

ITC grows **336 bytes**, 24,440 -> **24,776**. Cold code and rodata remain 3,500
and 1,603 bytes. Two cached floats add **80 state bytes across ten voices**.
This modest but repeated saving was retained; no extra waveform tables or heap
allocations are introduced. All 120 Wavesmith tests pass in debug and release,
repository/ABI checks and ARM relocation validation pass. The direct-target
version is deployed via the supplied scratch script; telemetry is stopped and
ports closed after measurement. Listening-sensitive smoothing changes remain
separate from this accepted arithmetic optimization.


## Round 9: two-sample pitch smoothing, accepted after listening

Same ten-voice stereo CV workload at 48 kHz / 24 frames. Each run used 100
seconds after five seconds warmup, with no missed reports or metadata changes.

| Build | Callback-body cycles | Setup cycles | Render/mix cycles |
|---|---:|---:|---:|
| round7-direct-repeat | 36,266 | 8,079 | 21,955 |
| round9-paired | 35,562 | 8,059 | 21,264 |
| round9-control | 36,265 | 8,070 | 21,964 |
| round9-paired-repeat | 35,315 | 7,828 | 21,248 |

The original repeated within 0.01%. Pair smoothing saved **1.94%** initially
and **2.62%** on repeat; render-only savings repeated at about 3.2%, while setup
shifted about 230 cycles between loads. A conservative interpretation is about
**2% additional whole-callback savings**, not CPU-meter percentage points.

The prototype computes a two-sample exponential endpoint with Q30 coefficient
alpha + alpha*(1-alpha), then uses a linear midpoint. New targets, sample-rate
changes and silent retunes cancel pending endpoints. Level smoothing, waveform
playback and existing target pitches remain unchanged. No input deadband or
band quantization is introduced. This approximates transient pitch tracking;
settled pitch remains the same.

122 Wavesmith tests pass in debug/release, with repository/ABI/static-placement
and relocation checks passing. New tests bound endpoint/midpoint/integrated
phase errors against the original cfg(test) integer recurrence, and check
monotonicity, no overshoot, settling and stale-endpoint cancellation. The older
float reference comparison now includes an explicit midpoint-curvature error
term instead of falsely requiring the unchanged trajectory.

ITC grows **176 bytes**, 24,776 -> **24,952**; cold code 3,500 and rodata 1,603
are unchanged. State grows **80 bytes**, 2,328 -> **2,408**, across ten voices.
No additional waveform tables or heap allocations are introduced. The user
reported CV glides and MIDI pitch bends sounded fine, so this improvement was
accepted and committed separately before investigating shared smoothing.


## Round 10: shared CV smoothing, rejected

The remaining candidate shared one normalized, two-sample-smoothed CV factor
curve across voices on an input bus, then applied each voice's cached tuning
scale and independent phase. Joining was limited to approximately one part per
million of the current phase increment, and range/level/gate/shape-transition
cases retained individual smoothing. Interleaved input groups replayed a cached
chunk-start snapshot so a bus was never advanced twice in the same chunk.

Initial deployment attempts failed during file transfer while another MIDI
monitor held the connection. The user stopped that monitor and recreated
scratch. All results below are fresh measurements of that recreated preset;
they must not be compared directly with the earlier round10-base measurement.
Only the supplied deployment script was used. Each resumed run captured 100
seconds after five seconds warmup, with ten CV voices, stereo, 48 kHz and
24 frames. Reports matched these metadata with no gaps. No notes or parameter
changes were sent by the profiling tools.

| Build | Callback-body cycles | Setup cycles | Render/mix cycles |
|---|---:|---:|---:|
| round10-resume-base | 38,134 | 8,054 | 23,778 |
| round10-resume-shared | 36,641 | 8,068 | 22,155 |
| round10-resume-control | 35,621 | 8,052 | 21,264 |

The first comparison suggested **3.92% fewer whole-callback cycles**, but the
returning accepted build was **6.59% faster than the first baseline**. Relative
to that returning control the shared candidate was **2.86% slower**.
Most variation was in rendering, where eligibility for cached-selection fast
paths changes. These measurements do not establish a reliable shared-smoothing
gain; the initial apparent reduction is not retained as a performance claim.

The candidate also cost **2,096 additional ITC bytes**, 24,952 -> 27,048 (+8.4%),
**112 cold-code bytes**, 3,500 -> 3,612, and **400 instance state bytes**,
2,408 -> 2,808; rodata remained 1,603 bytes. That is a weak trade-off against the
accepted optimizations even before the returning-control regression.

125 candidate tests passed in debug and release, plus repository/ABI/static
placement and relocation checks. A native ten-detuned-voice, changing-CV test
executed the shared path for 353,360 of 360,000 voice samples. Other tests
checked curve reuse, input ordering, finite output, monotonic settling and
tuning-change fallback. Those functional results do not establish lower CPU
cost or acceptable audio; no human listening approval was requested for this
candidate because it failed the performance/code-size gate.

The shared-smoothing code and its candidate-only tests were removed, preserving
all accepted changes including the user-approved paired smoothing (103dce5).
The restored source passes all **122 Wavesmith tests** and ARM relocation
validation, and the accepted build is deployed with the recreated scratch
preset. Streaming is disabled and monitor ports are closed. Candidate source,
patch, binary and raw measurements remain only under ignored
`target/wavesmith-profiling/`. No shared-smoothing implementation is retained.


## Refactor review: shared oscillator setup

CV and MIDI preparation now share sample-rate coefficient setup, pitch-moving
flag maintenance and level-target updates. The helper-only build has identical
ARM instruction sections to the preceding build (only source-location data
changed), with unchanged ITC/cold-code/rodata: 24,952/3,500/1,603 bytes.

The original one-sample smoothing coefficient is now local during setup and
stored only in test builds for the independent reference implementation.
Production oscillators shrink from 80 to 76 bytes, reducing ARM State from
2,408 to 2,368 bytes (framework storage from 2,448 to 2,408). ITC becomes
24,968 bytes (+16); cold code and rodata are unchanged. No table memory changes.

All 122 Wavesmith tests and ARM relocation validation pass. Each device capture
lasted 100 seconds after five seconds warmup, with ten CV voices, stereo,
48 kHz and 24 frames, and no missing telemetry reports.

| Build | Callback-body cycles | Setup cycles | Render/mix cycles |
|---|---:|---:|---:|
| refactor-base | 38,129 | 8,054 | 23,775 |
| refactor-setup (identical instructions) | 35,565 | 8,051 | 21,209 |
| refactor-state (40 fewer RAM bytes) | 35,667 | 8,010 | 21,333 |

The helper-only reload changed callback timing by 6.7% despite identical
instructions, illustrating the workload variability already observed in round
10. The state reduction differs from that run by only +0.29%; no CPU improvement
or regression is established. This refactor is retained for one implementation
of shared setup and smaller production state. Metrics and independent numerical
reference tests remain in place. Raw captures and comparison binaries live only
under ignored `target/wavesmith-refactor/`.


## Refactor review: explicit render eligibility

The callback's tuning array now holds octave offsets throughout setup. A
separate existing boolean array controls rendering and telemetry voice counts,
including MIDI release tails. CV no longer overwrites its tuning entries with
1.0, and MIDI no longer overwrites them with frequency values solely to use
zero as a disabled sentinel. This removes mixed units and duplicate eligibility
bookkeeping without adding arrays or persistent state.

All 122 Wavesmith tests pass in debug and release, the repository-wide test
suite passes, and the ARM object passes relocation validation. ITC decreases
by 36 bytes relative to the state-only refactor, 24,968 -> 24,932. Instance RAM,
cold code and rodata are unchanged.

A 100-second device capture after five seconds warmup recorded 35,426 callback
cycles, 7,846 setup cycles and 21,262 render/mix cycles, versus 35,667/8,010/21,333
for the state-only build. The apparent 0.68% callback reduction is small relative
to the variation demonstrated above; this is retained for clearer control flow
and reduced instruction size, with no established overall CPU gain.

The returning state-only control measured 35,419 callback cycles, 7,787 setup
cycles and 21,308 render/mix cycles. It was 0.69% faster than its first run,
matching the apparent callback gain of the refactor. This reinforces that no
whole-callback speed improvement is established by these captures.


## Refactor review: shared wavetable interpolation

One explicitly inlined Block::interpolate implementation replaces equivalent
closures in the fixed-selection and moving-pitch renderers and the scalar
Engine lookup. Table selection and lane binding remain outside the fast sample
loops. Arithmetic and sample ordering are preserved; the ARM diff shows
register scheduling changes in scalar lookup and two exchanged multiply
operands in playback, with unchanged section sizes and no helper call.

All 122 tests pass in debug and optimized release builds, the full repository
suite passes, and ARM relocation validation passes. Independent DSP reference
methods, in-place initialization and both profiling mechanisms remain intact.
No additional runtime arrays or allocations are introduced.

| Build | Callback-body cycles | Setup cycles | Render/mix cycles |
|---|---:|---:|---:|
| explicit eligibility | 35,426 | 7,846 | 21,262 |
| shared interpolation | 35,419 | 7,846 | 21,256 |
| returning state-only control | 35,419 | 7,787 | 21,308 |

Each run used the same 100-second capture protocol and workload described
above, with no missing reports. Interpolation differs by only -0.02% from the
preceding build; it is retained to remove duplicated interpolation code, with
no CPU claim. Its ITC, cold code, rodata and instance state match the preceding
build: 24,932 / 3,500 / 1,603 / 2,368 bytes respectively.

The complete refactor pass reduces ITC by 20 bytes and instance State by
40 bytes versus a40d3c3. Parameter metadata, waveform/table storage, MIDI/CV
pitch behavior, level control, UI and the metrics protocol are unchanged.
The existing separation of MIDI, stereo, levels and UI remains useful; broader
render-loop merging or file splitting would add churn without a demonstrated
size/performance benefit. Test-only historical DSP routines remain as numerical
references rather than being deleted as dead firmware code.
