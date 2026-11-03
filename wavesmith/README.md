# Wavesmith prototype

A CV or MIDI oscillator built from 2–32 curved segments, initially equal-width.
Setup specifications are Segments (2–32, default 8) and Voices (1–10, default 1).
Every instance uses six 256-sample anti-alias tables per bank. Configured voices
share the same waveform and table banks. Shape edits work while audio plays.
Use the NT's respecification workflow to change either count.

## Controls

- Left encoder: select segment. In Segment mode, selection wraps from the last
  segment to the first and from the first to the last, including larger turns.
- Press a potentiometer to reset its current control. In Segment mode, boundary
  presses zero the selected endpoint and the curve press restores its default
  zero curve. Warp and Tuning controls use their declared parameter defaults.
  The starting waveform's nonzero boundary defaults are unchanged. Reset occurs
  once per press; pickup resumes from the current physical position.
  MIDI-disabled Pitch stays inactive.
- Right encoder press: cycle Segment → Warp → Tuning → Segment.
- Segment mode (initial): left pot edits the selected segment's starting boundary
  level, centre pot edits its curve, right pot edits its ending boundary level.
  Levels and curves range from −100% to +100%; curve 0 is linear. Positive Curve
  bows upward and negative Curve bows downward. Stronger bows reflect at the
  whole waveform's −100%/+100% limits, never at the segment's endpoint levels.
- Warp mode: left pot edits Warp Position (0–255), centre pot edits Warp Amount
  (−100% to +100%), right pot edits Smoothing (0–100%). Shape triggers, output Level,
  and Pitch remain available on parameter pages.
- Tuning mode: left pot edits Pitch (MIDI note 0–127), centre pot edits Fine tune
  (−100 to +100 cents), right pot edits Level (0–100%). The top row displays
  these three values instead of the warp controls. Pitch is displayed as a note
  name and octave matching the NT (36 = C2, 37 = C#2), while its stored value
  remains a MIDI note number.
- Right encoder rotation is unused. Left encoder selects the segment in all modes.

The top row follows the current pot assignments: Left / Curve / Right in Segment
mode, Warp Pos / Warp Amt / Smooth in Warp mode, and Pitch / Fine / Level in
Tuning mode. Segment values read the selected segment's shared boundary parameters
and curve parameter, including the shared seam. All labels use regular-sized text. The
graph occupies y=14–42 with its zero axis at y=28, leaving the help footer fixed.
All three modes draw their header at full brightness (15) over a dim background
bar (2). Warp and Tuning modes hide the segment selection and boundary handles.
The waveform stays at full brightness in all modes to preserve its antialiasing
range. Segment mode restores the selected
segment's shading and handles. The help
footer contains one row of encoder hints: Push: Mode on the right in every mode,
and Turn: Select on the left only in Segment mode. Pot assignments are shown
only in the header. Switching modes preserves the selected segment and parameter values;
smart pickup resumes from the current knob positions without a value jump.

Pots use the C++ plugins' smart soft takeover: the remaining virtual range is
mapped onto the remaining knob travel, so either physical stop always reaches
the corresponding minimum or maximum, even after selecting another segment.
Fractional movement is retained for slow turns, and unexpected position jumps
are ignored to resynchronize after visiting another screen. Adjacent segments
share a single boundary level and always meet. For example, the right pot on
Segment 3 and the left pot on Segment 4 both edit Boundary 03. The beginning and
end of the complete waveform share Boundary 00, editable from either endpoint.
It defaults to zero but can be set anywhere from −100% to +100%; both ends always
meet at that value when the waveform repeats.
Warp Position ranges from 0 to 255 (default 128); Warp Amount defaults to zero. Positive Warp Amount repels boundary
positions from the warp position; negative Warp Amount attracts them. Both waveform ends
remain fixed and boundaries retain their order, level, and curve identity. A
monotone rational map limits the effective attraction/repulsion to 95% internally
at full control travel, preventing zero-width segments. Warp Position can sit at either
edge. A four-pixel black strip with a centered two-pixel full-brightness core
marks it only in Warp mode, drawn over the waveform and clamped on screen
at either edge. Grid dividers,
selection shading and endpoint handles follow the redistributed boundaries.
Zero Warp Amount retains the original equal-width geometry.
Equal start/end levels produce a flat segment only at Curve 0; nonzero Curve can
create humps and folded lobes between equal endpoints. The curve is quadratic
Bezier-shaped before folding, with an added midpoint bow of up to ±400% at Curve
±100 at the default Fold Limit of 2. Both endpoints remain exact. Folding is
continuous reflection, not clipping.

Fold Limit is a global 0–2 parameter (default 2) on the Shape page.
At zero, Curve still bends the segment using a quadratic Bezier control point
bounded by the global amplitude rails; it introduces no folds or hard clipping.
At limits 1–2, the available midpoint bow becomes ±(2 × limit) at Curve ±100,
so each sampled curve value needs no more than the allowed number of reflections
to return inside the rails. The value is a maximum, not a forced fold count;
Curve zero remains linear at every setting. Default 2 preserves the previous
curve arithmetic exactly. The maximum is two to keep folding useful at the
256-sample table resolution. Overall smoothing and final
anti-alias filtering still follow the curve/folding stage. Changing Fold Limit
rebuilds the waveform through the existing chunked, crossfaded path, without
adding FFTs or waveform buffers. Shape triggers leave Fold Limit unchanged.

The footer is always visible at y=50–63, matching Window Comparator's reserved
area. The custom display contains global values, the graph and help footer. The draw
callback returns true to suppress the NT's standard parameter line, since the
waveform editor uses the pots and encoders in this view.
The bright, antialiased trace shows the defined shape with optional creative
smoothing and brief edit crossfades. Only this bright editor trace is drawn;
it is independent of the individual voices' pitch-dependent audio filtering.
The display uses the NT's
native antialiased float-coordinate drawing API; this does not filter the audio.
All three modes show the vertical segment dividers as one-pixel dots spaced three
pixels apart, at brightness 1 (the lowest nonzero level). The horizontal zero
axis stays solid at brightness 1, and the thicker warp position marker stays solid.
This keeps Warp Position/Warp Amount changes visible with less prominent dividers.
Full-brightness boundary handles in Segment mode mark both
ends of the selected segment above the curves. The bright trace is independent
of pitch and the audio path's automatic
anti-alias filtering. At Smooth = 0 it evaluates the exact drawn curves; above
zero it reads a separate creative-smoothing table, preserving the boundary levels.

## Parameters and routing

Oscillator page: MIDI Channel (Off, All, or 1–16), Pitch (MIDI note number,
default A3/220 Hz), Fine tune in cents, and Level (default 50%, up to nominal
±5 V at 100%).

Shape follows Output, in this order: Warp Position, Warp Amount, Smoothing,
Fold Limit, Sine Boundaries, Triangle Boundaries, Square Boundaries, Sawtooth
Boundaries, Zero Curves, Randomize Curves, Randomize Boundaries, Randomize All.
The warp controls are ordinary modulatable host parameters. The underlying parameter list follows the same order as the pages.

- Randomize All replaces every boundary level and segment curve with
  pseudorandom values from −100% to +100%.
- Randomize Boundaries changes only boundary levels, including the shared seam.
- Randomize Curves changes only segment curves.
- Zero Curves sets every segment curve to zero.
- Sine Boundaries sets the boundaries to a sampled sinusoid, starting at its
  negative peak so even two-segment instances remain bipolar.
- Triangle Boundaries sets sampled triangle levels, starting at the negative peak.
- Square Boundaries sets the first half's boundary levels negative and the second
  half positive. The joins ramp across the available segments.
- Sawtooth Boundaries sets a rising ramp from the negative seam to the positive
  last boundary, then returns to the seam across the final segment.

The four waveform presets change boundary levels only, including the shared seam.
They preserve curves, smoothing and warp settings; use Zero Curves for the cleanest
linear-segment approximation. With very few segments, the four shapes can coincide
or approximate their target coarsely. These presets add no FFTs or audio buffers.

Each trigger ranges from 0 to 1 and acts only on its own rising edge. Return it
to 0, then 1 to act again; holding it at 1 does not repeat or affect the other
triggers' edge detection. All actions preserve pitch, routing, level, smoothing,
and warp settings. Generated values remain ordinary editable host parameters
and are saved with the preset.
The Output page follows Oscillator and contains shared Output and Output mode
(add/replace).
Each configured voice has its own Voice 1, Voice 2, etc. page, containing Pitch CV,
Transpose, and Fine tune in that order. Parameter names are shared; the API's
parameter UI prefix identifies the voice as V1: through V10:. Voice pages share
a navigation group so switching voices retains the selected parameter row.
Transpose ranges from -48 to +48 semitones; per-voice Fine tune ranges from -100
to +100 cents. Both offsets default to zero. Output routing remains shared.
Output defaults to bus 13 / physical output 1. Output None disables all voice
CV reading, frequency conversion, phase advancement and sample generation;
shared waveform rebuilding and edit fades can still finish while silent.
An enabled output sums every configured voice without normalization or limiting.
Replace clears the destination once before mixing the whole pool; Add preserves
the existing bus signal. Level is applied to each voice as before.

In CV mode every configured voice plays continuously. Its pitch is shared Pitch
plus Pitch CV, shared Fine tune, voice Transpose, and voice Fine tune. A Pitch CV
of None contributes zero volts, so voices can form manual chords with Transpose
or detuned stacks with their Fine tune offsets. CV values are snapshotted
before the shared output is cleared or written, including when an input is also
the destination bus. Each voice has its own phase, pitch/gain slew and anti-alias
selection. Persistent frequency caches and steady playback remain active.
The waveform builder, 24 KiB table buffers and crossfade clock are shared, so
more voices do not add FFT work or duplicate tables. Small playback/MIDI arrays
reserve the maximum ten states; only the specified voice pool is processed,
and parameter metadata includes only the specified CV inputs.

MIDI Channel defaults to Off, preserving continuous CV/manual playback. Set All
or a channel number to control every enabled voice with MIDI instead. In MIDI
mode, Pitch CV inputs, manual Pitch, and per-voice Transpose are ignored. Shared
Fine tune, per-voice Fine tune, and Level still apply. The ignored parameters are
grayed out through the NT API while MIDI is enabled and restored when MIDI is
Off, without changing their stored values. In the custom Tuning view, Pitch is
dim and its pot is inactive in MIDI mode; Fine and Level remain editable.
The Voices specification determines the 1–10-note MIDI pool. Unassigned MIDI
voices remain silent; when no notes are held, the shared output is silent after
release fades finish.

Note On uses an idle voice first, then a releasing voice, then steals the oldest
held note. Repeated Note On for an already-held key retriggers that voice. Note
Off and velocity-zero Note On release the matching channel/note; releasing a
stolen key does not stop its replacement. About 5 ms attack/release fades reduce
clicks. A stolen voice fades out before starting its new pitch from silence.
Pitch bend uses the full 14-bit MIDI value with a fixed +/-2-semitone range.
Bend is tracked independently per MIDI channel, affects held/releasing and new
notes, and is reset when MIDI Channel changes. It combines with both fine-tune
offsets; CV mode is unaffected. Bend-range RPN messages are not handled.
Velocity does not change loudness. Sustain and aftertouch are not handled. All Notes Off releases notes on its channel; All Sound Off silences them
immediately. Changing MIDI Channel clears held notes, as does setting shared Output to None. MIDI allocation adds only small voice states, without extra waveform
tables or FFT work.

With N segments there are N level parameters named Boundary 00, Boundary 01,
etc., and N independently named Curve parameters. One Waveform page lists them
in waveform order: Boundary 00, Curve 01, Boundary 01, Curve 02, Boundary 02,
and so on, ending with Curve N and Boundary 00. Each interior boundary appears
once and is still shared by its neighboring segments in the custom editor.
One editable parameter supplies both endpoint rows. There are four shared
pages (Oscillator, Output, Shape, Waveform) plus one page per configured voice and
the NT's own Common page. Segment count does not add pages; the maximum is 14
plugin pages with ten voices. This avoids the firmware's page-count issue at
high segment counts.
All numeric shape parameters are host-visible;
dedicated segment modulation routing is deferred. Ordinary parameter values are
stored by the NT host with the preset; editor selection is temporary.

## Rendering

Segment evaluation first bows and folds the source shape. Creative Smoothing
and its boundary anchoring follow folding; audio anti-alias filtering follows
Smoothing. The bright trace shows the folded source with Smoothing. The existing Curve parameter names,
indices and ranges are unchanged, but nonzero values now describe this bow/fold
model rather than the earlier rational easing; old shapes are not converted.

Every waveform edit builds six 256-sample audio tables, with harmonic limits
127, 32, 8, 4, 2, and 1. The most detailed table retains up to 127 harmonics;
the simplest retains the fundamental and DC. Two banks permit live edits,
so memory holds twelve audio tables plus two creative-smoothing display tables.
Audio tables are generated in pairs: one complex inverse FFT produces two real
waveforms, extracted from its real and imaginary components. Three inverse FFTs
generate all six tables, reusing the existing scratch buffer. Source sampling,
creative Smoothing, boundary anchoring, update cadence and edit crossfades are
unchanged. Playback selects adjacent tables using a binary search and blends between
them only when the richer table's highest harmonic is between 47% and 49% of the
audio sample rate (22.56–23.52 kHz at 48 kHz). The richest table is fully removed
before any of its harmonics exceeds that limit. This narrow, late crossfade
retains more detail than the original broad octave crossfade, though larger
bands of still-safe harmonics can disappear as tables change compared with the
eight-table bank, particularly at low and middle pitches. With six
tables, around F7 the eighth harmonic
is retained fully. Pitch changes reuse the bank without rebuilding it.
Steady playback uses at most two table lookups per waveform sample, with four
during a brief waveform-edit crossfade. There is no per-sample harmonic synthesis.
Each voice caches its anti-alias table selection and blend until its actual,
slewed pitch changes by 1 mV equivalent at 1 V/oct (1.2 cents) from the last
accepted selection. This applies equally to CV and MIDI playback. Actual phase
and pitch smoothing still follow every pitch change; the deadband affects only
anti-alias filtering. Small changes accumulate against the cached pitch, so
slow glides still update the filter. Stable notes reuse the selection without
repeating its table search and blend calculation on every sample. This adds one
cached float per voice and no waveform buffers or FFT work.
Voice caches also persist across audio blocks: note/CV-to-frequency conversion
runs only when the exact combined pitch changes, and phase-increment and slew
setup runs only when frequency or sample rate changes. Level smoothing restarts
only when Level changes. Pitch and level recurrences stop when they reach their
existing floating-point fixed point, without snapping or rounding the pitch.
Once a voice is settled, its MIDI gate is fully open (or MIDI is Off), and no
waveform crossfade is running, a simpler block renderer uses the cached integer
phase step, gain, table selection and blend. It resolves the current table slices
outside the sample loop and interpolates the source samples directly. A changing
pitch/level, note attack/release/steal, or waveform crossfade uses the dynamic
path. Table slices are rebound each block so a published waveform bank is used
immediately, without storing long-lived references to host memory. Stable CV
benefits too; even tiny CV changes invalidate the frequency cache. This adds
only small per-voice state and no waveform buffers or FFTs.
Low notes retain more detail; high notes become smoother and may become quieter
if most of the drawn shape's energy was in the removed harmonics. This reduces
aliasing substantially, but finite source sampling and linear table interpolation
still leave some residual error; it is not a guarantee of completely alias-free audio.

When Smooth is above zero, a 256-sample FFT applies the requested smoothing.
The control proportionally blends the original shape toward the fully smoothed
shape, spreading the change across its full travel instead of sweeping a filter
cutoff that becomes ineffective early. Maximum smoothing is unchanged.
Boundary constraints still limit how
smooth a shape with strongly alternating boundary levels can become.
Creative smoothing still anchors the source's shared boundaries, including the
seam, before the anti-alias filter. The final filtered tables deliberately do not
restore those anchors, which would reintroduce out-of-band harmonics. Both ends
of playback remain periodic and meet at the same value. The bright editor trace
uses the shape before the final anti-alias filter, so raising pitch changes the
audio without changing the drawing. Audio and display share the brief edit
crossfade. Boundary parameters, randomization, modulation targets and presets
retain the drawn source shape.

A five-millisecond crossfade prevents abrupt edit clicks without resetting phase.
That short transition also applies when enabling or disabling smoothing. A table
set without smoothing takes seven source callbacks (prepare geometry, four
64-sample chunks, FFT half 1, FFT half 2) plus four callbacks per pair of tables:
nineteen callbacks with the fixed six tables.
Every forward and inverse FFT runs in
two dedicated callbacks, each performing four of its eight radix-2 stages.
Reordering belongs to the first half; inverse normalization to the second.
Sampling, filtering, preview storage, spectrum copying, and table storage run
separately from the FFT halves. Each of the two boundary-correction passes takes one
callback to measure errors and apply DC, followed by callbacks that each apply
up to four harmonics. With six tables and zero Warp Amount, a smoothed build takes
30 callbacks at eight segments, or 36 at 32 segments. At 24 frames and 48 kHz,
those are 15 ms and 18 ms, versus 9.5 ms without smoothing, followed by the fade.
Nonzero Warp Amount with smoothing adds four 64-sample resampling callbacks. Smoothing and
boundary correction run in equal-width segment coordinates, retaining the
existing cached correction weights. The finished shape is then stretched into
the new segment positions before the final anti-alias FFT. The bright preview
evaluates that stretch directly from the retained smoothing table, keeping
anchors visible even when a compressed segment spans less than one audio-table
sample. Strong compression can lose detail in the fixed 256-sample audio tables.
Shifted boundary positions and reciprocal widths are computed once per rebuild,
then samples walk the cached segments in order rather than repeatedly searching
and recalculating positions. The geometry cache fits in the unused end of the
existing FFT lookup buffer, separate from its twiddles and smoothing gains.
Sampling and resampling each generate at most 64 samples per callback. At 24
frames and 48 kHz, 32 segments and six tables take 20 ms to rebuild at nonzero
Warp Amount with smoothing (40 callbacks), plus the 5 ms crossfade.
This reduces work per callback without adding buffers or changing the correction.
Continuous edits are coalesced without restarting an
unfinished build. The old complete table set plays until the new set is ready.
Startup is silent during the first build, then fades in. This is interactive
control-rate editing; it is not an audio-rate waveform modulation engine.

Audio/preview tables and build buffers occupy separately constructed SRAM blocks
with a fixed buffer footprint of 24 KiB, excluding the small instance state
and parameter metadata.
Two additional blocks cache the boundary-correction analysis weights once at
startup for the selected segment count. Harmonic correction reuses the FFT's
half-cycle lookup instead of regenerating sine/cosine recurrences on every edit.
The fixed smoothing filter response fits in the unused half of the FFT lookup
buffer. Smoothing still applies both boundary-correction passes with the same
correction order; these caches do not depend on the current shape or Smooth value.
Two real-valued tables are packed into each block. Each table pair takes four
callbacks: prepare the paired spectrum, inverse FFT half 1, inverse FFT half 2,
and store the pair. Each build callback performs at most half an FFT, retaining
paired table generation without adding buffers.
This keeps
large temporaries off the firmware stack. No memory allocation or locks occur
during audio rendering. There is no automatic whole-wave shift or normalization.
Smoothed shapes can ring beyond their requested levels; ±5 V at full Level is
nominal, not a hard peak limit. Asymmetric waves can have DC offset.

## Build and test

The audio bank is fixed at six tables. Voices ranges from 1–10 and defaults to 1.
The page order is Oscillator, Output, Shape, Voice 1 through Voice N, Waveform.
Underlying parameter ordinals now follow that same order: 18 shared controls,
three parameters per configured voice, then interleaved boundary/curve values.
Boundary 00 is one shared seam parameter shown again at the end of Waveform.

This revision changes parameter ordinals. Recreate earlier instances/presets and
review CV-to-parameter mappings; old ordinal-based values/mappings are not
migrated automatically. Changing segment count now preserves the indices of
shared controls, voices, and remaining waveform controls. Changing voice count
preserves shared controls and remaining voice controls, but shifts the waveform
block; it does not solve arbitrary respecification across both counts.

From the repository's devcontainer:

Instance state is initialized field by field in its final allocation through the
API's safe `RegionInitializer` and `initialize_region!` macro. This avoids large
whole-state temporaries on the firmware stack. Both the direct-initialization
diagnostic and the integrated safe version saved successfully on hardware after
the ordinary constructor crashed. All Rust plugins now use required direct state
and preallocated-region initialization hooks. Wavesmith continues to forbid unsafe
code. This changes no parameters or preset data.

```sh
make plugins/wavesmith.o
cargo test -p wavesmith --lib
```

Copy `plugins/wavesmith.o` to the NT's plugin folder using your normal deployment
procedure. Requires API-v14-capable firmware. Hardware listening, control feel,
and CPU load still need validation on a Disting NT. Preset libraries,
independent segment-width editing, and per-boundary smoothing are deferred.
