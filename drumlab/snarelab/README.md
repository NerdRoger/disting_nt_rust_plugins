# SnareLab

SnareLab is a mono snare drum synthesizer for the disting NT. Build a hit from
a pitched body, a layer of snare wires, and an optional rim transient. Each
layer has its own model and level, and every trigger plays the combined sound.

## Getting started

1. Add SnareLab with **Max Layers** set to 3 and **Output Buffers** set to 6.
2. Patch a trigger or gate to **Input 2** and listen to **Output 2**. These are
   the default assignments. Change **Trigger** or **Output** on the SnareLab
   page to use another input, output, or internal bus.
3. The default patch uses **Two-Stage** on layer 1, **Reson. Wires** on layer 2,
   and **Rim Shot** on layer 3. Start with **Head Pitch** on layer 1 and
   **Wire Brightness** and **Level** on layer 2.
4. Solo a layer while adjusting its tone, then turn Solo off to hear the mix.
   To audition a new combination, confirm **Randomize Layers** on the SnareLab
   page.

A rising edge at 1 V starts a hit. A held gate plays one hit and rearms when the
signal falls to 0.1 V or below.

## Specifications

| Specification | Range | Default | Use |
| --- | --- | --- | --- |
| **Max Layers** | 2–5 | 3 | Sets how many layer pages are available. With two layers, the default patch starts with body and wires. Layers 4 and 5 start at Off. |
| **Output Buffers** | 2–10 | 6 | Sets how many rendered waveform versions can be retained while tails play and edits are rendered. More buffers use more DRAM. |

## Controls

| SnareLab page | Function |
| --- | --- |
| **Trigger**, **Output** | Select the trigger source and mono destination. |
| **Output Mode** | **Replace** writes the snare signal to the destination bus and is the default. **Add** mixes it with audio already on that bus. |
| **Master Drive**, **Master Gain** | Drive adds saturation to the mix. Gain adjusts the overall level after processing. |
| **Randomize Layers** | After confirmation, selects a body on layer 1 and wires on layer 2, and may add a rim on layer 3 if available. It changes model settings and levels, clears Solo and Mute, and sets any remaining layers to Off. |

Each layer has a page named after its current model. Any layer can use any model.
Confirming a different **Model** loads that model’s defaults and shows its
controls. Choosing **Off** disables the layer. **Randomize** on a layer varies
the current model’s controls around its defaults while keeping its model and
level. Use **Level** to balance the mix, **Solo** to hear the soloed layers, and
**Mute** to silence a layer.

### Models

| Model | Role | Sound and starting controls |
| --- | --- | --- |
| **Pitch Sweep** | Body | A pitched head with a downward sweep and optional click. Try Head Pitch, Sweep Depth, and Body Decay. |
| **Resonator** | Body | A struck resonant head with a pitch drift in the tail. Try Excitation, Exciter Width, and Tail Pitch Drift. |
| **Modal Body** | Body | A struck head with multiple resonances. Try Upper Modes, Damping, and Strike Position. |
| **Two-Stage** | Body | A higher-pitched strike that gives way to the sustained head. Try Strike Pitch, Head Pitch, and Handoff Time. |
| **Snare Wires** | Wires | A filtered noise layer with adjustable onset and tail. Try Wire Brightness, Wire Delay, and Wire Decay. |
| **Reson. Wires** | Wires | A rising wire layer with a pitched tail ring. Try Wire Rise, Wire Length, and Tail Ring Level. |
| **Sweep Wires** | Wires | A more textured wire sound with a swept filter. Try Filter Q, Grain, and Wire Brightness. |
| **Rim Shot** | Rim | A short pitched rim and a bright click. Try Rim Pitch, Snap Level, and Click Focus. |

## Waveform and playback

The waveform screen shows the rendered mix and its peak level across a 750 ms
time axis. Edits render in the background. Until the new waveform is ready,
new triggers use the last completed version, and hits already playing can
finish their tails.

The buffer meter covers the last five seconds. **Used** is the peak number of
waveform slots occupied at once. **Waits** counts times an edit had to wait for
a free slot. If waits recur while you edit and trigger the snare, try a higher
**Output Buffers** setting. Save the NT preset to keep your sound.

## Installation

Copy `snarelab.o` to `/programs/plug-ins/` on the NT’s SD card. Rescan plugins or
restart the module, then add SnareLab. Use firmware that supports plugin API 15
for background rendering.

From the repository root in the project’s devcontainer, run
`make plugins/snarelab.o` to build it. See the
[repository build guide](../../README.md#getting-started) for setup.
