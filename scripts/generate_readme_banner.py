from __future__ import annotations

import math
from pathlib import Path
from typing import Iterable

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont


WIDTH = 1200
HEIGHT = 240
TEXT_SHIFT_X = 0
TEXT_SHIFT_Y = -17
FONT_PATH = Path("C:/Windows/Fonts/consolab.ttf")
OUTPUT = Path(__file__).resolve().parent.parent / "assets" / "cortex-header.gif"
WORD = "CORTEX"
LETTER_GAP = "  "

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


def build_letter_layout(word: str, gap: str = "  ") -> list[tuple[str, int]]:
    layout: list[tuple[str, int]] = []
    cursor = 0
    for index, letter in enumerate(word):
        if index:
            cursor += len(gap)
        layout.append((letter, cursor))
        cursor += max(len(row) for row in LETTER_FORMS[letter])
    return layout


ASCII_LINES = build_ascii_word(WORD, LETTER_GAP)
LETTER_LAYOUT = build_letter_layout(WORD, LETTER_GAP)

BG = (45, 47, 54, 255)
PANEL = (50, 53, 60, 255)
PANEL_ACCENT = (64, 67, 76, 255)
PANEL_EDGE = (255, 255, 255, 16)
SHADOW = (19, 7, 30, 98)
UNDERLAY = (80, 48, 118, 118)
PURPLE_TOP = (109, 74, 154, 255)
PURPLE_BOTTOM = (82, 45, 128, 255)
SWEEP = (244, 230, 255, 245)
BORDER_GLOW = (118, 72, 170, 82)
BORDER_CORE = (151, 110, 196, 126)
WAKE_FLASH = (246, 233, 255, 230)


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


def draw_letter_masks(size: tuple[int, int]) -> list[Image.Image]:
    font = make_font()
    _, line_height = text_metrics(font)
    char_width = font.getlength("M")
    base_x, base_y = text_origin()
    masks: list[Image.Image] = []

    for letter, start_col in LETTER_LAYOUT:
        mask = Image.new("L", size, 0)
        draw = ImageDraw.Draw(mask)
        x = int(base_x + start_col * char_width)
        for row_index, line in enumerate(LETTER_FORMS[letter]):
            draw.text((x, base_y + row_index * line_height), line, fill=255, font=font)
        masks.append(mask)

    return masks


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
    left: int, top: int, right: int, bottom: int, radius: int, steps: int = 18
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
    segment = 34
    start = int((frame_index / max(1, frame_count)) * total)

    glow = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    glow_draw = ImageDraw.Draw(glow)
    core = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    core_draw = ImageDraw.Draw(core)

    for i in range(segment - 1):
        a = points[(start + i) % total]
        b = points[(start + i + 1) % total]
        t = i / max(1, segment - 1)
        strength = ((math.cos(t * math.pi) + 1.0) / 2.0) ** 1.15
        glow_alpha = int(BORDER_GLOW[3] * strength)
        core_alpha = int(BORDER_CORE[3] * (strength**1.2))
        glow_draw.line((a, b), fill=(BORDER_GLOW[0], BORDER_GLOW[1], BORDER_GLOW[2], glow_alpha), width=6)
        core_draw.line((a, b), fill=(BORDER_CORE[0], BORDER_CORE[1], BORDER_CORE[2], core_alpha), width=2)

    image.alpha_composite(glow.filter(ImageFilter.GaussianBlur(4)))
    image.alpha_composite(core)


def offset_mask(mask: Image.Image, dx: int, dy: int, blur_radius: float = 0) -> Image.Image:
    shifted = Image.new("L", mask.size, 0)
    shifted.paste(mask, (dx, dy))
    if blur_radius:
        shifted = shifted.filter(ImageFilter.GaussianBlur(blur_radius))
    return shifted


def wake_masks(letter_masks: list[Image.Image], frame_index: int) -> tuple[Image.Image, Image.Image]:
    base = Image.new("L", (WIDTH, HEIGHT), 0)
    flash = Image.new("L", (WIDTH, HEIGHT), 0)

    for index, letter_mask in enumerate(letter_masks):
        phase = frame_index - index * 2
        if phase < 0:
            base_alpha = 0
            flash_alpha = 0
        elif phase == 0:
            base_alpha = 46
            flash_alpha = 120
        elif phase == 1:
            base_alpha = 188
            flash_alpha = 255
        elif phase == 2:
            base_alpha = 112
            flash_alpha = 72
        else:
            base_alpha = 255
            flash_alpha = 0

        if base_alpha:
            base.paste(base_alpha, mask=letter_mask)
        if flash_alpha:
            flash.paste(flash_alpha, mask=letter_mask)

    return base, flash


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
    half_band = 64.0

    for y in range(HEIGHT):
        skew = y * 0.42
        for x in range(WIDTH):
            distance = abs((x - skew) - center)
            if distance > half_band:
                continue
            strength = 1.0 - (distance / half_band)
            alpha = int(SWEEP[3] * (strength**2.6))
            pixels[x, y] = (SWEEP[0], SWEEP[1], SWEEP[2], alpha)

    clipped = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    clipped.paste(overlay, mask=mask)
    return clipped


def frame_sequence() -> Iterable[Image.Image]:
    frame_count = 32
    text_mask = draw_text_mask((WIDTH, HEIGHT))
    letter_masks = draw_letter_masks((WIDTH, HEIGHT))
    fill_mask = text_mask.filter(ImageFilter.MaxFilter(7))
    shadow_mask = draw_text_mask((WIDTH, HEIGHT), offset=(8, 8)).filter(ImageFilter.GaussianBlur(2.5))
    glow_mask = text_mask.filter(ImageFilter.GaussianBlur(4))
    base_text = vertical_gradient((WIDTH, HEIGHT), PURPLE_TOP, PURPLE_BOTTOM)
    startup_frames = len(letter_masks) * 2 + 4
    shimmer_start = startup_frames - 1

    for frame_index in range(frame_count):
        frame = make_background(frame_index, frame_count)

        current_text_mask = text_mask
        current_fill_mask = fill_mask
        current_shadow_mask = shadow_mask
        current_glow_mask = glow_mask
        flash_mask = None

        if frame_index < startup_frames:
            current_text_mask, flash_mask = wake_masks(letter_masks, frame_index)
            current_fill_mask = current_text_mask.filter(ImageFilter.MaxFilter(7))
            current_shadow_mask = offset_mask(current_text_mask, 8, 8, blur_radius=2.5)
            current_glow_mask = current_text_mask.filter(ImageFilter.GaussianBlur(4))

        shadow = Image.new("RGBA", (WIDTH, HEIGHT), SHADOW)
        frame.alpha_composite(Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0)))
        frame.paste(shadow, mask=current_shadow_mask)

        underlay = Image.new("RGBA", (WIDTH, HEIGHT), UNDERLAY)
        frame.paste(underlay, mask=current_fill_mask)

        glow_strength = 24 + (10 if frame_index in {0, 15} else 0)
        glow = Image.new("RGBA", (WIDTH, HEIGHT), (132, 78, 206, glow_strength))
        frame.paste(glow, mask=current_glow_mask)

        text_layer = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
        text_layer.paste(base_text, mask=current_text_mask)
        frame.alpha_composite(text_layer)

        if flash_mask is not None:
            flash_overlay = Image.new("RGBA", (WIDTH, HEIGHT), WAKE_FLASH)
            frame = ImageChops.screen(frame, Image.composite(flash_overlay, Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0)), flash_mask))

        if frame_index >= shimmer_start:
            highlight = sweep_overlay(text_mask, frame_index - shimmer_start, frame_count - shimmer_start + 1)
            frame = ImageChops.screen(frame, highlight)

        yield frame.convert("P", palette=Image.Palette.ADAPTIVE, dither=Image.Dither.NONE)


def main() -> None:
    frames = list(frame_sequence())
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    frames[0].save(
        OUTPUT,
        save_all=True,
        append_images=frames[1:],
        duration=115,
        loop=0,
        optimize=False,
        disposal=2,
    )
    print(f"wrote {OUTPUT}")


if __name__ == "__main__":
    main()
