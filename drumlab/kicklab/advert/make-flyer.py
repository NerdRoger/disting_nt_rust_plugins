"""Regenerate this advert with Python and Pillow.

Install dependencies with python -m pip install -r requirements.txt.
Edit the copy and model list below, or replace either source screenshot with
a 512 × 128 or 1024 × 256 capture from the disting NT.
"""

import argparse
import math
import os
import random
from pathlib import Path

from PIL import Image, ImageColor, ImageDraw, ImageFont


SLUG = "kicklab"
TITLE = "KICKLAB"
HOOK = "From deep thump to dirty attack."
FEATURE = "Layer up to 8 sounds into one hit."
ACCENT = "#FF9F6E"
GRADIENT = "#2D191E"
SEED = 23
MODELS = (
    ("Pitch Sweep", "A pitch drop with a separate click."),
    ("Modal Drum", "A resonant body and adjustable beater."),
    ("Bit Logic", "Clocked logic with a crunchy edge."),
    ("Grain Cloud", "Short grains with pitch and timing spread."),
    ("Hard Sync", "A sync transient over a pitched tail."),
    ("Resonator", "A short strike excites a tuned ring."),
    ("Two Op. FM", "An FM attack with its own decay."),
    ("Noise Sweep", "A swept band of filtered noise."),
    ("Ring Mod", "A pitched body with a metallic clang."),
    ("Wavefold", "A folded tone with a clean sub blend."),
    ("Delay Drum", "A strike into a tuned feedback loop."),
    ("Spect. Pulse", "A pitched body with a spectral snap."),
)

HERE = Path(__file__).resolve().parent
WIDTH = 1200
PAPER = "#F6F2EF"
SECONDARY = "#DAD2D7"
MUTED = "#ACABB8"


def load_fonts(directory):
    """Load the typefaces used in the original flyer."""
    files = {
        "mono": "consola.ttf",
        "mono_bold": "consolab.ttf",
        "body": "segoeui.ttf",
        "bold": "segoeuib.ttf",
        "title": "bahnschrift.ttf",
    }

    def font(face, size):
        try:
            result = ImageFont.truetype(str(directory / files[face]), size)
        except OSError as error:
            raise SystemExit(f"Cannot open {files[face]} in {directory}. Use --font-dir to set the font folder.") from error
        if face == "title":
            result.set_variation_by_name("Bold")
        return result

    return font


def background(height):
    angle = math.radians(61)
    dx, dy = math.cos(angle), math.sin(angle)
    extent = WIDTH * dx + height * dy
    top = ImageColor.getrgb(GRADIENT)
    bottom = ImageColor.getrgb("#0A0D15")
    palette = [
        tuple(round(a + (b - a) * step / 255) for a, b in zip(top, bottom))
        for step in range(256)
    ]
    x_projection = [x * dx for x in range(WIDTH)]
    ramp = Image.new("P", (WIDTH, height))
    ramp.putpalette([channel for color in palette for channel in color])
    ramp.frombytes(bytes(
        int(255 * (x + y * dy) / extent)
        for y in range(height)
        for x in x_projection
    ))

    picture = ramp.convert("RGBA")
    accents = Image.new("RGBA", picture.size)
    draw = ImageDraw.Draw(accents)
    rgb = ImageColor.getrgb(ACCENT)
    rng = random.Random(SEED)
    for _ in range(1700):
        draw.point((rng.randrange(WIDTH), rng.randrange(height)), fill=(*rgb, 30))
    if SLUG == "kicklab":
        for radius in range(62, 62 + 11 * 18, 18):
            draw.ellipse((1050 - radius, 244 - radius, 1050 + radius, 244 + radius),
                         outline=(*rgb, 36))
    else:
        for offset in range(0, 15 * 15, 15):
            draw.line((812 + offset, 354, 1020 + offset, 140), fill=(*rgb, 36))
            draw.line((812 + offset, 140, 1020 + offset, 354), fill=(*rgb, 36))
    return Image.alpha_composite(picture, accents)


def render(font_dir, output):
    rows = (len(MODELS) + 1) // 2
    grid_top, row_height = 562, 70
    footer = 552 + rows * row_height + 16
    image = background(footer + 138).convert("RGB")
    draw = ImageDraw.Draw(image)
    font = load_fonts(font_dir)
    rule = Image.blend(Image.new("RGB", (1, 1), "#0F121B"),
                       Image.new("RGB", (1, 1), ACCENT), 0.40).getpixel((0, 0))

    def text(x, y, value, face, size, color=PAPER, anchor="lt"):
        draw.text((x, y), value, font=font(face, size), fill=color, anchor=anchor)

    def divider(y):
        draw.line((72, y, 1128, y), fill=rule)
        draw.line((72, y, 148, y), fill=ACCENT, width=3)

    text(72, 62, "DISTING NT  /  EURORACK PLUGIN", "mono_bold", 23, ACCENT)
    text(1128, 65, "NT PLUGIN API 15", "mono", 19, MUTED, anchor="rt")
    divider(104)
    text(72, 119, TITLE, "title", 116)
    text(73, 247, HOOK, "bold", 42)
    text(75, 303, FEATURE, "body", 27, SECONDARY)

    for x, screen in ((72, "waveform"), (608, "layer-parameters")):
        draw.rectangle((x, 340, x + 520, 484), fill="#0B111B", outline=rule)
        draw.line((x, 340, x + 56, 340), fill=ACCENT, width=3)
        with Image.open(HERE / f"{SLUG}-{screen}.png") as screenshot:
            if screenshot.size not in ((512, 128), (1024, 256)):
                raise SystemExit(f"Unexpected dimensions for the {screen} screenshot: {screenshot.size}")
            preview = screenshot.convert("RGB").resize((512, 128), Image.Resampling.NEAREST)
            image.paste(preview, (x + 4, 348))

    text(73, 514, f"{len(MODELS)} SYNTHESIS MODELS", "mono_bold", 23, ACCENT)
    for index, (name, blurb) in enumerate(MODELS):
        column, row = divmod(index, rows)
        x, y = 73 + 555 * column, grid_top + row_height * row
        text(x, y, name, "bold", 26)
        text(x, y + 30, blurb, "body", 24, SECONDARY)

    divider(footer)
    text(73, footer + 37, "Build your own. Or roll the dice.", "bold", 37, ACCENT)
    text(75, footer + 88, "Randomize a layer or the whole hit.", "body", 26, SECONDARY)
    image.save(output, format="PNG", dpi=(96, 96))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=HERE / f"{SLUG}-discord-flyer.png")
    parser.add_argument("--font-dir", type=Path,
                        default=Path(os.environ.get("WINDIR", "C:/Windows")) / "Fonts")
    arguments = parser.parse_args()
    render(arguments.font_dir, arguments.output)


if __name__ == "__main__":
    main()
