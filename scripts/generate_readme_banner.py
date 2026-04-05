from __future__ import annotations

from pathlib import Path
from typing import Iterable

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont


WIDTH = 1200
HEIGHT = 240
PADDING_X = 56
PADDING_Y = 34
FONT_PATH = Path("C:/Windows/Fonts/consolab.ttf")
OUTPUT = Path(__file__).resolve().parent.parent / "assets" / "cortex-header.gif"

ASCII_LINES = [
    "  ______   ____   _____   _______   ______   __   __",
    " / ____| / __ \\ |  __ \\ |__   __| |  ____|  \\ \\ / /",
    "| |     | |  | || |__) |   | |    | |__      \\ V /",
    "| |     | |  | ||  _  /    | |    |  __|      > <",
    "| |____ | |__| || | \\ \\    | |    | |____    / . \\",
    " \\_____| \\____/ |_|  \\_\\   |_|    |______|  /_/ \\_\\",
]

BG = (44, 46, 54, 255)
PANEL = (54, 57, 66, 255)
GRID = (84, 88, 98, 16)
SHADOW = (40, 15, 58, 92)
UNDERLAY = (94, 53, 145, 108)
PURPLE_TOP = (151, 111, 214, 255)
PURPLE_BOTTOM = (85, 37, 131, 255)
SWEEP = (220, 196, 255, 235)


def make_font() -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(str(FONT_PATH), 31)


def text_metrics(font: ImageFont.FreeTypeFont) -> tuple[int, int]:
    box = font.getbbox("Ag")
    line_height = box[3] - box[1] + 6
    max_width = max(font.getlength(line) for line in ASCII_LINES)
    return int(max_width), line_height


def draw_text_mask(size: tuple[int, int], offset: tuple[int, int] = (0, 0)) -> Image.Image:
    font = make_font()
    _, line_height = text_metrics(font)
    mask = Image.new("L", size, 0)
    draw = ImageDraw.Draw(mask)
    x = PADDING_X + offset[0]
    y = PADDING_Y + offset[1]
    for index, line in enumerate(ASCII_LINES):
        draw.text((x, y + index * line_height), line, fill=255, font=font)
    return mask


def vertical_gradient(size: tuple[int, int], top: tuple[int, int, int, int], bottom: tuple[int, int, int, int]) -> Image.Image:
    width, height = size
    image = Image.new("RGBA", size)
    pixels = image.load()
    for y in range(height):
        t = y / max(1, height - 1)
        color = tuple(int(top[i] * (1 - t) + bottom[i] * t) for i in range(4))
        for x in range(width):
            pixels[x, y] = color
    return image


def make_background(frame_index: int, frame_count: int) -> Image.Image:
    image = Image.new("RGBA", (WIDTH, HEIGHT), BG)
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle((16, 16, WIDTH - 16, HEIGHT - 16), radius=12, fill=PANEL)

    shift = (frame_index * 8) % 24
    for y in range(0, HEIGHT, 6):
        alpha = 12 if ((y + shift) // 6) % 2 == 0 else 4
        draw.rectangle((0, y, WIDTH, y + 1), fill=(GRID[0], GRID[1], GRID[2], alpha))

    return image


def sweep_overlay(mask: Image.Image, frame_index: int, frame_count: int) -> Image.Image:
    overlay = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    pixels = overlay.load()
    center = -240 + (WIDTH + 480) * (frame_index / frame_count)
    half_band = 120.0

    for y in range(HEIGHT):
        skew = y * 0.68
        for x in range(WIDTH):
            distance = abs((x - skew) - center)
            if distance > half_band:
                continue
            strength = 1.0 - (distance / half_band)
            alpha = int(SWEEP[3] * (strength**1.6))
            pixels[x, y] = (SWEEP[0], SWEEP[1], SWEEP[2], alpha)

    clipped = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    clipped.paste(overlay, mask=mask)
    return clipped


def frame_sequence() -> Iterable[Image.Image]:
    frame_count = 18
    text_mask = draw_text_mask((WIDTH, HEIGHT))
    fill_mask = text_mask.filter(ImageFilter.MaxFilter(5))
    shadow_mask = draw_text_mask((WIDTH, HEIGHT), offset=(8, 8)).filter(ImageFilter.GaussianBlur(2.5))
    glow_mask = text_mask.filter(ImageFilter.GaussianBlur(5))
    base_text = vertical_gradient((WIDTH, HEIGHT), PURPLE_TOP, PURPLE_BOTTOM)

    for frame_index in range(frame_count):
        frame = make_background(frame_index, frame_count)

        shadow = Image.new("RGBA", (WIDTH, HEIGHT), SHADOW)
        frame.alpha_composite(Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0)))
        frame.paste(shadow, mask=shadow_mask)

        underlay = Image.new("RGBA", (WIDTH, HEIGHT), UNDERLAY)
        frame.paste(underlay, mask=fill_mask)

        glow_strength = 34 + (8 if frame_index in {0, 9} else 0)
        glow = Image.new("RGBA", (WIDTH, HEIGHT), (121, 68, 190, glow_strength))
        frame.paste(glow, mask=glow_mask)

        text_layer = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
        text_layer.paste(base_text, mask=text_mask)
        frame.alpha_composite(text_layer)

        highlight = sweep_overlay(text_mask, frame_index, frame_count)
        frame = ImageChops.screen(frame, highlight)

        yield frame.convert("P", palette=Image.Palette.ADAPTIVE)


def main() -> None:
    frames = list(frame_sequence())
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    frames[0].save(
        OUTPUT,
        save_all=True,
        append_images=frames[1:],
        duration=90,
        loop=0,
        optimize=False,
        disposal=2,
    )
    print(f"wrote {OUTPUT}")


if __name__ == "__main__":
    main()
