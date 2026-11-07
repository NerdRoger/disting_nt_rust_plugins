# Wavesmith

Wavesmith is a waveform-building oscillator for the disting NT. Draw a waveform
on the screen, bend and fold its curves, and play it with CV or MIDI. Up to 24
voices share the same shape, with independent tuning and panning for each voice.

## Overview

The waveform is divided into segments. Each segment joins two boundary levels,
and its Curve control changes the path between them. Neighboring segments share
the same boundary, so adjusting a join changes both sides together. The beginning
and end of the waveform also share a boundary, allowing each cycle to meet the
next at the same level.

You can change the shape while it plays. Start with a familiar waveform from the
Reshape page, edit individual segments, then use the Morph controls to stretch
the shape and soften or fold its curves.

When adding Wavesmith, choose these specifications:

| Specification | Description |
|--------|--------|
| `Voices` | Number of oscillator voices, from 1 to 24. Default: 1. Start with one voice, then add more when you want chords, detuning, or MIDI stacks. |
| `Segments` | Number of waveform segments, from 2 to 32. Default: 32. Fewer segments make broad shapes easier to edit; more give you finer control. |

A new instance starts with a triangle-like waveform, Pitch at C3, and Level at
−6 dB. Left Output defaults to physical Output 1, and every voice's Pitch CV
defaults to Input 1. MIDI Channel defaults to Off, so the oscillator plays
continuously in CV mode.

## Getting started

1. Add Wavesmith with one voice. Leave Segments at 32, or choose a smaller number
   if you would like fewer points to work with.
2. Listen to physical Output 1. Leave MIDI Channel set to Off for now.
3. Open the custom waveform screen. Turn the left encoder to select a segment.
4. Move the left and right pots to set its endpoint levels, and the centre pot
   to change its curve. The sound and display follow your edits.
5. Press the right encoder to reach the warp controls, then experiment with
   Warp Amount and Smoothing. Press it again for Pitch, Fine tune, and Level.

For a starting shape, open Reshape and apply Sine, Triangle, Square, or Sawtooth.
These set the boundary levels and clear all segment curves. They leave your
warp and smoothing settings alone, so set Warp Amount and Smoothing to zero if
you want to hear the starting shape without those changes.

## Waveform screen

The values across the top show what the three pots are currently controlling.
Press the right encoder to cycle through these modes:

| Mode | Left pot | Centre pot | Right pot |
|--------|--------|--------|--------|
| Segment | Left boundary level | Curve | Right boundary level |
| Warp | Warp Position | Warp Amount | Smoothing |
| Tuning | Pitch | Fine tune | Level |

Turn the left encoder to select a segment. In Segment mode, turning past the
last segment wraps to the first, and vice versa. The selected segment and its
endpoints are highlighted. Warp mode shows a vertical marker for Warp Position.
The zero line and segment dividers help you see the waveform's layout, and the
encoder hints remain visible at the bottom.

Press a pot to reset the value it controls. Boundary levels reset to zero, Curve
to zero, and the other controls to their defaults. Pot movement is ignored while
you press and for 100 ms after release, so releasing the button does not knock
the value away from its reset position. Normal editing then resumes from the
pot's current position. Switching modes or segments does not jump a value to
the physical pot position.

In MIDI mode, the shared Pitch control is dimmed and its pot is inactive. Fine
tune and Level still work.

### Boundaries and curves

Boundary levels range from −100% to +100%. For example, the right endpoint of
Segment 3 is the same point as the left endpoint of Segment 4. Editing either
one changes the same boundary.

Boundary 00 is the join between the last segment and the first. It can be set
to any level; it does not have to stay at zero.

Curve also ranges from −100% to +100%:

- At zero, the segment is a straight line between its endpoints.
- Positive values bow the curve upward; negative values bow it downward.
- With folding enabled, stronger curves can go beyond the waveform's upper or
  lower limit and reflect back into the shape. Even a segment with equal
  endpoint levels can have a hump or folded shape between them.

The bright trace shows your shape with Smoothing applied. Higher audio notes
may sound less bright than the drawing suggests because Wavesmith removes
harmonics that would otherwise alias at that pitch.

## Parameters

The pages appear in this order: Oscillator, Output, Morph, Reshape, the individual
Voice pages, and Waveform.

### Oscillator

| Parameter | Description |
|--------|--------|
| `MIDI Channel` | Off uses continuous CV/manual playback. All or a channel from 1–16 enables MIDI note control. Default: Off. |
| `Pitch` | Root note for CV/manual playback. Each voice's Pitch CV and Transpose are added to it. Default: C3. Ignored and grayed out in MIDI mode. |
| `Fine tune` | Shared tuning adjustment from −100 to +100 cents, applied in both CV and MIDI modes. Default: 0. |
| `Level` | Overall output level, from −60 to 0 dB. The bottom setting displays −inf and mutes the output. Default: −6 dB. |

### Output

| Parameter | Description |
|--------|--------|
| `Left Output` | Output or bus for the left channel, or the mono signal when only this output is assigned. Default: physical Output 1. |
| `Right Output` | Output or bus for the right channel. Default: None. Assign a different bus from Left Output to enable stereo. |
| `Output mode` | Add mixes Wavesmith with the signal already on the selected buses. Replace overwrites those buses with Wavesmith's output. Default: Add. |

One assigned output gives you mono through that output, whether it is Left or
Right. Assigning both outputs to the same bus also gives mono. Pan is grayed out
in mono, and its settings are restored when you return to stereo. Setting both
outputs to None silences Wavesmith.

In stereo, a centered voice is quieter in each individual channel than it is in
mono: its energy is spread across both channels. Wavesmith uses equal-power
panning to keep the overall balance more consistent as a voice moves across
the stereo field.

### Morph

| Parameter | Description |
|--------|--------|
| `Warp Position` | Centre of the waveform stretch/compression effect, from 0 to 255. Default: 128. Shown by the vertical marker in Warp mode. |
| `Warp Amount` | Redistributes the boundaries around Warp Position. Positive values push them away; negative values pull them toward it. Range: −100% to +100%. Zero restores equal-width segments. |
| `Smoothing` | Softens the waveform between its boundary levels, reducing sharp detail and brightness. Range: 0–100%. Default: 0%. |
| `Fold Limit` | Maximum folding allowed when bending a segment's curve, from 0 to 2. Zero keeps curves within the waveform limits; higher values allow reflection at the upper and lower limits. Default: 2. |

Warp changes where boundaries sit in the cycle, while keeping their order and
levels. Smoothing keeps the specified boundary levels, but can bend a previously
straight stretch between them. Fold Limit allows folds rather than forcing them;
a segment with Curve at zero stays straight at every Fold Limit setting.

### Reshape

These actions are useful starting points for a new sound. Change an action from
Ready to Apply and confirm it in the NT parameter interface. After it runs, it
returns to Ready automatically, so you can use it again without resetting it.

| Parameter | Description |
|--------|--------|
| `Sine` | Sets boundary levels along a sine wave and zeros all segment curves. |
| `Triangle` | Sets triangle-wave boundary levels and zeros all segment curves. |
| `Square` | Sets the first half of the boundary levels negative and the second half positive, and zeros all curves. |
| `Sawtooth` | Sets rising boundary levels followed by a return to the start, and zeros all curves. |
| `Zero Curves` | Straightens every segment without changing its boundary levels. |
| `Randomize Curves` | Randomizes the segment curves while keeping the boundary levels. |
| `Randomize Boundaries` | Randomizes the boundary levels while keeping the curves. |
| `Randomize All` | Randomizes both the boundary levels and curves. |

The four waveform presets approximate their shapes using your selected segment
count. Square and Sawtooth have sloped transitions rather than instantaneous
jumps, and Sine uses straight joins between its sampled levels. More segments
give a closer approximation. All Reshape actions preserve your tuning, routing,
level, warp, smoothing, and Fold Limit settings.

Actions can also be driven through the NT's parameter mappings. A direct mapped
change to 1 runs the action without the interactive confirmation; the action
then resets itself to 0.

### Voice pages

Each configured voice has its own page, with the same five controls. The voice
prefix identifies the voice in other parameter lists. Moving between Voice pages
keeps you on the same parameter row.

| Parameter | Description |
|--------|--------|
| `Pitch CV` | Input or bus for this voice's 1 V/oct pitch offset. None adds no CV offset. Default: Input 1. Ignored and grayed out in MIDI mode. |
| `Transpose` | Voice tuning offset from −48 to +48 semitones. Applies in CV and MIDI modes. Default: 0. |
| `Fine tune` | Voice tuning offset from −100 to +100 cents, added to the shared Fine tune setting. Applies in both modes. Default: 0. |
| `Pan` | Stereo position, from 0% left to 100% right. Default: 50% centre. Grayed out in mono. |
| `MIDI Voice Group` | None or Group 1–10. Voices in the same group play the same MIDI note. Grayed out and ignored in CV mode. |

Voices 1–10 default to Groups 1–10 respectively. Voices above 10 default to None;
assign them to a group if you want to use them in MIDI mode.

### Waveform

This page lists the shape in order: Boundary 00, Curve 01, Boundary 01, Curve 02,
and so on, ending with the last curve and Boundary 00 again. Both Boundary 00
entries control the same value. You can edit or map these parameters through
the standard NT controls as well as using the custom waveform screen.

Only the selected number of segments appears on the page. Additional unused
boundaries and curves may appear in mapping lists, but are grayed out and do not
change the current waveform.

## Playing with CV

Set MIDI Channel to Off. Every configured voice then plays continuously. Its
pitch combines the shared Pitch and Fine tune settings, its Pitch CV input at
1 V/oct, and its own Transpose and Fine tune offsets.

To thicken a sound, add several voices, give them the same Pitch CV input, and
apply small positive and negative Fine tune offsets. Assign separate left and
right outputs and spread their Pan settings for a wider sound.

For a chord from a single CV, try three voices with Transpose set to 0, +4, and
+7 semitones, all using the same Pitch CV. With a C root, that gives C, E, and G.
Set Pitch CV to None if you would like a voice to stay at its manually set pitch.
CV mode has no note gate or envelope; use an external VCA and envelope when you
want to articulate the output.

## Playing with MIDI

Set MIDI Channel to All or to your controller's channel. MIDI takes over note
control for all assigned voices; Pitch CV and the shared Pitch setting are
ignored. Notes sound while held, with short attack and release fades.

MIDI allocates notes to groups, rather than to individual oscillators. For
example, with six voices you could assign Voices 1–3 to Group 1 and Voices 4–6
to Group 2. That gives two-note polyphony, with three oscillators on each note.
Detune and pan the members of each group independently to create a stereo stack.

Available polyphony is the number of groups with at least one assigned voice,
up to ten notes. If all available groups are busy, the newest note steals the
oldest held note and all oscillators in that group follow the replacement.
A voice assigned to None does not play in MIDI mode; it still plays in CV mode.

Pitch bend is supported with a fixed range of ±2 semitones. Shared and per-voice
Fine tune, per-voice Transpose, and Pan remain available. Velocity does not
change loudness, and sustain pedal, aftertouch, and bend-range configuration
messages are not supported.

## Level and headroom

Wavesmith automatically reduces the combined level of multiple voices to help
keep a chord near the loudness of a single note. It also protects its own output
peaks at ±5 V. Different shapes and chords can still have different perceived
loudness, and peak protection may reduce the level of a dense chord.

The default −6 dB leaves useful headroom. At 0 dB there is less room for peaks,
so adding notes can engage more protection. In Add mode, audio already on the
bus is outside Wavesmith's level control; the combined bus can exceed ±5 V.

## Changing the specifications

Use the NT's Respecify function to change Voices or Segments. Changing Segments
preserves stored boundary and curve values; it changes which ones take part in
the waveform. Reducing Voices removes the trailing voices and their settings;
newly added voices receive their defaults. After changing either specification,
check the resulting shape and voice assignments.

Your shape and parameter settings are saved with the NT preset. Remember to save
the preset after finding a sound you want to keep.

## Installation and housekeeping

Copy `wavesmith.o` to `/programs/plug-ins/` on the NT's SD card, then rescan plugins
or restart the module and add Wavesmith. Use firmware that supports plugin API 14.

Report bugs and suggestions on [GitHub](https://github.com/NerdRoger/disting_nt_rust_plugins/issues).
For build instructions, implementation notes, and profiling, see
[Developer.md](Developer.md).
