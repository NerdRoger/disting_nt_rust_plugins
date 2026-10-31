# Wavesmith prototype

A free-running oscillator built from 2–32 equal-width curved segments. Select the
segment count in the algorithm's setup specifications when adding Wavesmith.
The NT allocates parameter layouts at construction: changing the count requires
replacing/recreating the instance. Shape edits work while audio plays.

## Controls

- Left encoder: select segment.
- Right encoder press: toggle Segment / Globals editing modes.
- Segment mode (initial): left pot edits the selected segment's starting boundary
  level, centre pot edits its curve, right pot edits its ending boundary level.
  Levels and curves range from −100% to +100%; curve 0 is linear.
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
Equal start/end levels produce a flat segment regardless of curve.

The footer is always visible at y=50–63, matching Window Comparator's reserved
area. The custom display contains global values, the graph and help footer. The draw
callback returns true to suppress the NT's standard parameter line, since the
waveform editor uses the pots and encoders in this view.
A single bright, antialiased graph shows the shape used for playback,
including optional smoothing and brief edit crossfades. The display uses the NT's
native antialiased float-coordinate drawing API; this does not filter the audio.
The grid uses solid lines at the lowest nonzero brightness (1) in Segment mode.
Globals mode hides the vertical segment dividers but retains the horizontal zero axis.
Full-brightness boundary handles in Segment mode mark both
ends of the selected segment above the curve. There is no separate unsmoothed
reference trace. At Smooth = 0, playback
evaluates the drawn segments directly at every audio sample: there is no harmonic
limit, sampled-wave approximation, or automatic anti-alias filtering. Applying
Smooth changes the displayed curve while preserving its shared boundary levels.

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

At zero smoothing, the oscillator evaluates the exact segment/curve formula at
its current phase. Raising pitch only changes phase speed; it does not change the
waveform. Sharp shapes can alias, as expected from direct digital playback.

When Smooth is above zero, a 256-sample FFT applies the requested smoothing.
The control proportionally blends the original shape toward the fully smoothed
shape, spreading the change across its full travel instead of sweeping a filter
cutoff that becomes ineffective early. Maximum smoothing is unchanged.
Boundary constraints still limit how
smooth a shape with strongly alternating boundary levels can become.
There is no additional harmonic cutoff or pitch-dependent filtering. Shared
boundaries and the shared seam remain anchored (within 0.1% table interpolation
tolerance). The audio and bright trace both read that same smoothed table.

A five-millisecond crossfade prevents abrupt edit clicks without resetting phase.
That short transition also applies when enabling or disabling smoothing. Direct
edits need no table build; smoothed edits take two audio callbacks to prepare
(about 2.7 ms at a 64-frame/48 kHz block), followed by the fade. Continuous edits
are coalesced without restarting an unfinished build. The initial waveform plays
directly, with a short gain fade-in. This is interactive control-rate editing.

Smoothing buffers occupy separately constructed SRAM blocks (about 10 KB), keeping
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
