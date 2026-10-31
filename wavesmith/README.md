# Wavesmith prototype

A free-running oscillator built from 2–32 equal-width curved segments. Select the
segment count and Anti-alias tables in the algorithm's setup specifications.
Anti-alias tables accepts 1–20, default 6. The NT allocates parameter layouts and
buffers at construction; use its respecification workflow to change these counts.
Shape edits work while audio plays.

One table disables automatic pitch-dependent anti-alias filtering and plays the
full-detail sampled cycle at every pitch; aliasing is expected for sharp shapes
at higher pitches. Creative Smoothing still works. With 2–20 tables, filtering
remains active and higher counts provide finer changes in harmonic content.
Only the chosen count is allocated, rather than reserving the maximum.

## Controls

- Left encoder: select segment.
- Right encoder press: toggle Segment / Globals editing modes.
- Segment mode (initial): left pot edits the selected segment's starting boundary
  level, centre pot edits its curve, right pot edits its ending boundary level.
  Levels and curves range from −100% to +100%; curve 0 is linear. Positive Curve
  bows upward and negative Curve bows downward. Stronger bows reflect at the
  whole waveform's −100%/+100% limits, never at the segment's endpoint levels.
- Globals mode: left pot edits Randomize (0/1), centre pot edits Level (0–100%),
  right pot edits Smoothing (0–100%). Turn Randomize up to trigger a new shape,
  down to rearm it, then up for another variation. Pitch remains on the Oscillator page.
- Right encoder rotation is unused. Left encoder selects the segment in either mode.

Randomize, Level and Smoothing use regular-sized text above the waveform. The
graph occupies y=14–42 with its zero axis at y=28, leaving the help footer fixed.
Globals
mode draws this header at full brightness (15) over a dim background (2), and hides the segment selection
and boundary handles. The waveform stays at full brightness in both modes to
preserve its antialiasing range. Segment mode dims the header (4) and restores the selected
segment's shading and handles. The help
footer shows the current pot assignments and the mode a right-encoder click will
select. Switching modes preserves the selected segment and parameter values;
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
Equal start/end levels produce a flat segment only at Curve 0; nonzero Curve can
create humps and folded lobes between equal endpoints. The curve is quadratic
Bezier-shaped before folding, with an added midpoint bow of up to ±400% at Curve
±100. Both endpoints remain exact. Folding is continuous reflection, not clipping.

The footer is always visible at y=50–63, matching Window Comparator's reserved
area. The custom display contains global values, the graph and help footer. The draw
callback returns true to suppress the NT's standard parameter line, since the
waveform editor uses the pots and encoders in this view.
The bright, antialiased trace shows the defined shape with optional creative
smoothing and brief edit crossfades. A dim audio trace (brightness 4) is drawn
behind it for development, showing pitch-dependent harmonic filtering and the
audio edit crossfade, before output Level scaling. The bright trace is always
drawn last so it stays in front wherever the traces meet. The display uses the NT's
native antialiased float-coordinate drawing API; this does not filter the audio.
The grid uses solid lines at the lowest nonzero brightness (1) in Segment mode.
Globals mode hides the vertical segment dividers but retains the horizontal zero axis.
Full-brightness boundary handles in Segment mode mark both
ends of the selected segment above the curves. The bright trace is independent
of pitch and the audio path's automatic
anti-alias filtering. At Smooth = 0 it evaluates the exact drawn curves; above
zero it reads a separate creative-smoothing table, preserving the boundary levels.

## Parameters and routing

Oscillator page: Pitch (MIDI note number, default A3/220 Hz), Fine tune in cents,
Smoothing, Level (default 50%, up to nominal ±5 V at 100%), and Randomize.
Randomize is a 0/1 trigger: changing it from 0 to 1 replaces all boundary
levels and segment curves with pseudorandom values from −100% to +100%. Return it
to 0, then 1 for another variation; holding it at 1 does not repeat. Endpoints
share the randomized Boundary 00, and pitch, routing, level and smoothing stay unchanged. The generated
values are ordinary editable host parameters and are saved with the preset.
Routing page: Output
(default bus 13 / physical output 1), Output mode (add/replace), and optional
1 V/oct Pitch CV bus (0 disables). Pitch CV is sampled once per audio block.
With N segments there are N level parameters named Boundary 00, Boundary 01,
etc., and N independently named Curve parameters. One Waveform page lists them
in waveform order: Boundary 00, Curve 01, Boundary 01, Curve 02, Boundary 02,
and so on, ending with Curve N and Boundary 00. Each interior boundary appears
once and is still shared by its neighboring segments in the custom editor.
One editable parameter supplies both endpoint rows. There are always three plugin pages
(Oscillator, Routing, Waveform), plus the NT's own Common page, for every segment
count. This avoids the firmware's page-count issue at high segment counts.
All numeric shape parameters are host-visible;
dedicated segment modulation routing is deferred. Ordinary parameter values are
stored by the NT host with the preset; editor selection is temporary.

## Rendering

Segment evaluation first bows and folds the source shape. Creative Smoothing
and its boundary anchoring follow folding; audio anti-alias filtering follows
Smoothing. The bright trace shows the folded source with Smoothing, and the dim
trace shows its filtered audio version. The existing Curve parameter names,
indices and ranges are unchanged, but nonzero values now describe this bow/fold
model rather than the earlier rational easing; old shapes are not converted.

Every waveform edit builds the specified number of 256-sample audio tables.
At the default six, harmonic limits remain 127, 32, 8, 4, 2, and 1. For counts
above one, limits are distinct and nested: increasing the count adds another
intermediate limit without removing those already present. The most detailed
table retains up to 127 harmonics and the simplest retains the fundamental and
DC. One-table mode instead retains the entire sampled cycle, including the
table's Nyquist bin, with no pitch-dependent cutoff or table crossfade.
Two banks permit live edits, so memory holds twice the specified number of audio
tables, plus two creative-smoothing display tables.
Audio tables are generated in pairs: one complex inverse FFT produces two real
waveforms, extracted from its real and imaginary components. Odd table counts
use a final unpaired transform. This reduces table-generation inverse FFTs from
N to ceil(N/2), reusing the existing scratch buffer. Source sampling, creative
Smoothing, boundary anchoring, update cadence and edit crossfades are unchanged.
With at least two tables, playback selects adjacent tables using a binary search and blends between
them only when the richer table's highest harmonic is between 47% and 49% of the
audio sample rate (22.56–23.52 kHz at 48 kHz). The richest table is fully removed
before any of its harmonics exceeds that limit. This narrow, late crossfade
retains more detail than the original broad octave crossfade, though larger
bands of still-safe harmonics can disappear as tables change compared with the
eight-table bank, particularly at low and middle pitches. At the default six
tables, around F7 the eighth harmonic
is retained fully. Pitch changes reuse the bank without rebuilding it.
Steady playback uses at most two table lookups per waveform sample, with four
during a brief waveform-edit crossfade. There is no per-sample harmonic synthesis.
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
set without smoothing takes three source callbacks (sample, FFT half 1, FFT
half 2) plus four callbacks per pair of tables: seven callbacks at 1 table,
fifteen at the default 6, and 43 at 20. Every forward and inverse FFT runs in
two dedicated callbacks, each performing four of its eight radix-2 stages.
Reordering belongs to the first half; inverse normalization to the second.
Sampling, filtering, preview storage, spectrum copying, and table storage run
separately from the FFT halves. Each of the two boundary-correction passes takes one
callback to measure errors and apply DC, followed by callbacks that each apply
up to four harmonics. With six tables, a smoothed build takes 26 callbacks at
eight segments, or 32 at 32 segments. At 64 frames and 48 kHz, those are
about 34.7 ms and 42.7 ms, versus 20 ms without smoothing, followed by the fade.
This reduces work per callback without adding buffers or changing the correction.
Continuous edits are coalesced without restarting an
unfinished build. The old complete table set plays until the new set is ready.
Startup is silent during the first build, then fades in. This is interactive
control-rate editing; it is not an audio-rate waveform modulation engine.

Audio/preview tables and build buffers occupy separately constructed SRAM blocks
with a buffer footprint of (2 × tables + 12) KiB: 14 KiB at 1 table, 24 KiB at 6,
and 52 KiB at 20, excluding the small instance state and parameter metadata.
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

This shared-boundary prototype changes the host parameter layout from the earlier
independent Start/End version. Recreate existing Wavesmith instances after loading
the updated plugin; saved shapes from the earlier layout are not migrated.
The single Waveform page retains the shared-boundary version's parameter indices
and values; it only changes how the host displays them.
Randomize is appended after the existing parameters, preserving their indices.
Boundary 00 reuses the former fixed-zero parameter, so this change also preserves
existing parameter indices and defaults to the previous zero seam.

From the repository's devcontainer:

```sh
make plugins/wavesmith.o
cargo test -p wavesmith --lib
```

Copy `plugins/wavesmith.o` to the NT's plugin folder using your normal deployment
procedure. Requires API-v14-capable firmware. Hardware listening, control feel,
and CPU load still need validation on a Disting NT. Preset libraries,
variable segment widths, and per-boundary smoothing are deferred.
