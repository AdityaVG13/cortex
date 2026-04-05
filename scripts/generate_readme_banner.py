from __future__ import annotations

import math
from pathlib import Path
from typing import Iterable

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont


WIDTH = 1200
HEIGHT = 240
TEXT_SHIFT_X = 0
TEXT_SHIFT_Y = -19
FONT_PATH = Path("C:/Windows/Fonts/consolab.ttf")
OUTPUT = Path(__file__).resolve().parent.parent / "assets" / "cortex-header.gif"

LETTER_FORMS = {
    "C": [
        "  _____ ",
        " / ____|",
        "| |     ",
        "| |     ",
        "| |____ ",
        " \\_____|",
    ],
    "O": [
        "  ____  ",
        " / __ \\ ",
        "| |  | |",
        "| |  | |",
        "| |__| |",
        " \\____/ ",
    ],
    "R": [
        " _____  ",
        "|  __ \\ ",
        "| |__) |",
        "|  _  / ",
        "| | \\ \\ ",
        "|_|  \\_\\",
    ],
    "T": [
        " _______ ",
        "|__   __|",
        "   | |   ",
        "   | |   ",
        "   | |   ",
        "   |_|   ",
    ],
    "E": [
        " ______ ",
        "|  ____|",
        "| |__   ",
        "|  __|  ",
        "| |____ ",
        "|______|",
    ],
    "X": [
        "__   __",
        "\\ \\ / /",
        " \\ V / ",
        "  > <  ",
        " / . \\ ",
        "/_/ \\_\\",
    ],
}


def build_ascii_word(word: str, gap: str = "  ") -> list[str]:
    rows = [""] * 6
    for index, letter in enumerate(word):
        glyph = LETTER_FORMS[letter]
        for row_index in range(6):
            if index:
                rows[row_index] += gap
            rows[row_index] += glyph[row_index]
    return rows


ASCII_LINES = build_ascii_word("CORTEX")

BG = (45, 47, 54, 255)
PANEL = (50, 53, 60, 255)
PANEL_ACCENT = (64, 67, 76, 255)
PANEL_EDGE = (255, 255, 255, 16)
SHADOW = (29, 12, 45, 96)
UNDERLAY = (94, 53, 145, 116)
PURPLE_TOP = (151, 111, 214, 255)
PURPLE_BOTTOM = (85, 37, 131, 255)
SWEEP = (220, 196, 255, 235)
BORDER_GLOW = (133, 86, 203, 150)
BORDER_CORE = (178, 134, 236, 220)


def make_font() -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(str(FONT_PATH), 31)


def text_metrics(font: ImageFont.FreeTypeFont) -> tuple[int, int]:
    box = font.getbbox("Ag")
    line_height = box[3] - box[1] + 6
    max_width = max(font.getlength(line) for line in ASCII_LINES)
    return int(max_width), line_height


def text_origin(offset: tuple[int, int] = (0, 0)) -> tuple[int, int]:
    font = make_font()
    text_width, line_height = text_metrics(font)
    text_height = line_height * len(ASCII_LINES)
    x = int((WIDTH - text_width) / 2) + TEXT_SHIFT_X + offset[0]
    y = int((HEIGHT - text_height) / 2) + TEXT_SHIFT_Y + offset[1]
    return x, y


def draw_text_mask(size: tuple[int, int], offset: tuple[int, int] = (0, 0)) -> Image.Image:
    font = make_font()
    _, line_height = text_metrics(font)
    mask = Image.new("L", size, 0)
    draw = ImageDraw.Draw(mask)
    x, y = text_origin(offset)
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


def rounded_rect_points(
    left: int, top: int, right: int, bottom: int, radius: int, steps: int = 12
) -> list[tuple[float, float]]:
    pts: list[tuple[float, float]] = []

    def add_line(x1: float, y1: float, x2: float, y2: float, count: int) -> None:
        for i in range(count):
            t = i / max(1, count - 1)
            pts.append((x1 + (x2 - x1) * t, y1 + (y2 - y1) * t))

    def add_arc(cx: float, cy: float, start_deg: float, end_deg: float, count: int) -> None:
        for i in range(count):
            t = i / max(1, count - 1)
            angle = math.radians(start_deg + (end_deg - start_deg) * t)
            pts.append((cx + radius * math.cos(angle), cy + radius * math.sin(angle)))

    add_line(left + radius, top, right - radius, top, 60)
    add_arc(right - radius, top + radius, -90, 0, steps)
    add_line(right, top + radius, right, bottom - radius, 28)
    add_arc(right - radius, bottom - radius, 0, 90, steps)
    add_line(right - radius, bottom, left + radius, bottom, 60)
    add_arc(left + radius, bottom - radius, 90, 180, steps)
    add_line(left, bottom - radius, left, top + radius, 28)
    add_arc(left + radius, top + radius, 180, 270, steps)
    return pts


def draw_border_runner(image: Image.Image, frame_index: int, frame_count: int) -> None:
    left, top, right, bottom, radius = 20, 20, WIDTH - 20, HEIGHT - 20, 16
    points = rounded_rect_points(left, top, right, bottom, radius)
    total = len(points)
    segment = 64
    start = int((frame_index / max(1, frame_count)) * total)

    glow = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    glow_draw = ImageDraw.Draw(glow)
    core = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    core_draw = ImageDraw.Draw(core)

    for i in range(segment - 1):
        a = points[(start + i) % total]
        b = points[(start + i + 1) % total]
        strength = 1.0 - (i / max(1, segment - 1))
        glow_alpha = int(BORDER_GLOW[3] * (strength**1.35))
        core_alpha = int(BORDER_CORE[3] * (strength**1.55))
        glow_draw.line((a, b), fill=(BORDER_GLOW[0], BORDER_GLOW[1], BORDER_GLOW[2], glow_alpha), width=6)
        core_draw.line((a, b), fill=(BORDER_CORE[0], BORDER_CORE[1], BORDER_CORE[2], core_alpha), width=2)

    image.alpha_composite(glow.filter(ImageFilter.GaussianBlur(3)))
    image.alpha_composite(core)


def make_background(frame_index: int, frame_count: int) -> Image.Image:
    image = Image.new("RGBA", (WIDTH, HEIGHT), BG)
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle((18, 18, WIDTH - 18, HEIGHT - 18), radius=16, fill=PANEL)
    draw.rounded_rectangle((18, 18, WIDTH - 18, HEIGHT - 18), radius=16, outline=PANEL_EDGE, width=1)
    draw.line((34, 44, WIDTH - 34, 44), fill=PANEL_ACCENT, width=1)
    draw.line((34, HEIGHT - 42, WIDTH - 34, HEIGHT - 42), fill=(32, 34, 39, 255), width=1)
    draw_border_runner(image, frame_index, frame_count)

    return image


def sweep_overlay(mask: Image.Image, frame_index: int, frame_count: int) -> Image.Image:
    overlay = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    pixels = overlay.load()
    center = -180 + (WIDTH + 360) * (frame_index / frame_count)
    half_band = 72.0

    for y in range(HEIGHT):
        skew = y * 0.42
        for x in range(WIDTH):
            distance = abs((x - skew) - center)
            if distance > half_band:
                continue
            strength = 1.0 - (distance / half_band)
            alpha = int(SWEEP[3] * (strength**2.25))
            pixels[x, y] = (SWEEP[0], SWEEP[1], SWEEP[2], alpha)

    clipped = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    clipped.paste(overlay, mask=mask)
    return clipped


def frame_sequence() -> Iterable[Image.Image]:
    frame_count = 18
    text_mask = draw_text_mask((WIDTH, HEIGHT))
    fill_mask = text_mask.filter(ImageFilter.MaxFilter(7))
    shadow_mask = draw_text_mask((WIDTH, HEIGHT), offset=(8, 8)).filter(ImageFilter.GaussianBlur(2.5))
    glow_mask = text_mask.filter(ImageFilter.GaussianBlur(4))
    base_text = vertical_gradient((WIDTH, HEIGHT), PURPLE_TOP, PURPLE_BOTTOM)

    for frame_index in range(frame_count):
        frame = make_background(frame_index, frame_count)

        shadow = Image.new("RGBA", (WIDTH, HEIGHT), SHADOW)
        frame.alpha_composite(Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0)))
        frame.paste(shadow, mask=shadow_mask)

        underlay = Image.new("RGBA", (WIDTH, HEIGHT), UNDERLAY)
        frame.paste(underlay, mask=fill_mask)

        glow_strength = 28 + (8 if frame_index in {0, 9} else 0)
        glow = Image.new("RGBA", (WIDTH, HEIGHT), (121, 68, 190, glow_strength))
        frame.paste(glow, mask=glow_mask)

        text_layer = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
        text_layer.paste(base_text, mask=text_mask)
        frame.alpha_composite(text_layer)

        highlight = sweep_overlay(text_mask, frame_index, frame_count)
        frame = ImageChops.screen(frame, highlight)

        yield frame.convert("P", palette=Image.Palette.ADAPTIVE, dither=Image.Dither.NONE)


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
