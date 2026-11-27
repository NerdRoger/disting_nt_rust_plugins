# KickLab

KickLab is a mono kick drum synthesizer for the disting NT. Start with a single
pitched kick, or layer a low body with a beater, noise, or a metallic transient.
Every trigger plays the combined sound.

## Getting started

1. Add KickLab with **Max Layers** set to 1 and **Output Buffers** set to 6.
2. Patch a trigger or gate to **Input 1** and listen to **Output 1**. These are
   the default assignments. Change **Trigger** or **Output** on the KickLab page
   to use another input, output, or internal bus.
3. Open **1-Pitch Sweep**. Adjust **Tail Tune** and **Body Decay** for the body,
   then try **Sweep Depth** and **Click Level** to shape the attack.
4. For a layered kick, use **Respecify** to add a layer. Choose a model on that
   layer and use **Level**, **Solo**, and **Mute** to hear how it fits.

A rising edge at 1 V starts a hit. A held gate plays one hit and rearms when the
signal falls to 0.1 V or below.

## Specifications

| Specification | Range | Default | Use |
| --- | --- | --- | --- |
| **Max Layers** | 1–8 | 1 | Sets how many layer pages are available. Layer 1 starts at Pitch Sweep, and additional layers start at Off. |
| **Output Buffers** | 2–10 | 6 | Sets how many rendered waveform versions can be retained while tails play and edits are rendered. More buffers use more DRAM. |

## Controls

| KickLab page | Function |
| --- | --- |
| **Trigger**, **Output** | Select the trigger source and mono destination. |
| **Output Mode** | **Replace** writes the kick signal to the destination bus and is the default. **Add** mixes it with audio already on that bus. |
| **Low-pass** | Filters the combined layers. It ranges from 40 Hz to 20 kHz and defaults to Off. |
| **Master Drive**, **Master Gain** | Drive adds saturation to the mix. Gain adjusts the overall level after processing. |
| **Randomize Layers** | After confirmation, chooses models, levels, and model settings across the layers and clears Solo and Mute. Layers that roll Off are placed at the end. |

Each layer has a page named after its current model. Confirming a different
**Model** loads that model’s defaults and shows its controls. Choosing **Off**
disables the layer. **Randomize** on a layer changes the current model’s
controls while keeping its model and level. **Level** sets how much of that
layer enters the mix. **Solo** restricts the mix to soloed layers, and **Mute**
silences an individual layer.

### Models

| Model | Sound and starting controls |
| --- | --- |
| **Pitch Sweep** | Classic pitch-dropping kick with a separate click. Start with Tail Tune, Body Decay, and Click Level. |
| **Modal Drum** | A struck body with multiple resonant modes and a beater. Try Upper Modes, Damping, and Beater Hardness. |
| **Bit Logic** | A clocked digital kick with selectable logic operations. Try Sample Clock, Ramp Bits, and Logic Operation. |
| **Grain Cloud** | A textured kick built from short grains. Try Grain Size, Grain Rate, and Pitch Spread. |
| **Hard Sync** | A pitched kick with a sync transient. Try Sync Bend, Sync Decay, and Tail Ratio. |
| **Resonator** | A tuned ring driven by a short excitation. Try Excitation, Exciter Width, and Tone Cutoff. |
| **Two Op. FM** | A kick with a frequency-modulated attack. Try FM Amount, Modulator Ratio, and FM Decay. |
| **Noise Sweep** | A swept band of noise. Try Tail Center, Filter Q, and Grain. |
| **Ring Mod** | A pitched body with a metallic clang. Try Lower Osc Ratio, Clang Level, and Clang Decay. |
| **Wavefold** | A folded kick with a clean sub blend. Try Fold Amount, Fold Decay, and Clean Sub Blend. |
| **Delay Drum** | A tuned feedback loop excited by a strike. Try Loop Cutoff, Strike Mix, and Strike Width. |
| **Spect. Pulse** | A pitched body with a shaped spectral snap. Try Snap Center, Snap Bandwidth, and Dispersion. |

## Waveform and playback

The waveform screen shows the rendered mix and its peak level across a
one-second time axis. Edits render in the background. Until the new waveform is
ready, new triggers use the last completed version, and hits already playing
can finish their tails.

The buffer meter covers the last five seconds. **Used** is the peak number of
waveform slots occupied at once. **Waits** counts times an edit had to wait for
a free slot. If waits recur while you edit and trigger long kicks, try a higher
**Output Buffers** setting. Save the NT preset to keep your sound.

## Installation

Copy `kicklab.o` to `/programs/plug-ins/` on the NT’s SD card. Rescan plugins or
restart the module, then add KickLab. Use firmware that supports plugin API 15
for background rendering.

From the repository root in the project’s devcontainer, run
`make plugins/kicklab.o` to build it. See the
[repository build guide](../../README.md#getting-started) for setup.
