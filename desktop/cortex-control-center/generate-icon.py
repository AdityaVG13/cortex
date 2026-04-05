"""Generate Cortex icon v4 — Big bold C, white nodes branching right."""
from PIL import Image, ImageDraw, ImageFont
import math
import os


def draw_cortex_icon(size: int) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    cx, cy = size / 2, size / 2
    r = size * 0.46

    # Background circle — deep indigo
    for i in range(int(r + 2), 0, -1):
        t = i / r
        cr = int(18 * (1 - t * 0.5))
        cg = int(12 * (1 - t * 0.4))
        cb = int(50 * (1 - t * 0.3))
        draw.ellipse([cx - i, cy - i, cx + i, cy + i], fill=(cr, cg, cb, 255))

    # Subtle border
    ring_w = max(2, size // 140)
    draw.ellipse(
        [cx - r, cy - r, cx + r, cy + r],
        outline=(80, 120, 240, 60),
        width=ring_w,
    )

    # --- Big bold white C, left-center, nearly filling the circle height ---
    font_size = int(size * 0.58)
    font = None
    for font_name in ["segoeuib.ttf", "arialbd.ttf", "segoeui.ttf"]:
        try:
            font = ImageFont.truetype(f"C:/Windows/Fonts/{font_name}", font_size)
            break
        except OSError:
            continue

    # Position: left side, vertically centered
    bbox = draw.textbbox((0, 0), "C", font=font)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    # C sits in the left ~45% of the circle
    tx = cx * 0.42 - tw / 2 - bbox[0]
    ty = cy - th / 2 - bbox[1]

    # C center and radius for placing branch origins
    c_optical_x = cx * 0.42
    c_optical_y = cy
    c_arc_radius = th * 0.44

    # --- Branch origin points along the C's opening (right side of C) ---
    # The C opens to the right. Branches sprout from the opening and outer curve.
    branch_origins = []

    # Points along the open side of C (top-right to bottom-right, ~330 to 30 deg)
    for deg in [345, 0, 15]:
        rad = math.radians(deg)
        ox = c_optical_x + c_arc_radius * math.cos(rad)
        oy = c_optical_y - c_arc_radius * math.sin(rad)
        branch_origins.append((ox, oy, "open"))

    # Points along the outer curve of C (top, right-top, etc)
    for deg in [60, 90, 120, 160, 200, 240, 270, 300]:
        rad = math.radians(deg)
        ox = c_optical_x + c_arc_radius * 1.05 * math.cos(rad)
        oy = c_optical_y - c_arc_radius * 1.05 * math.sin(rad)
        branch_origins.append((ox, oy, "curve"))

    # --- Generate branching nodes ---
    nodes = []  # (x, y, size_class)  size_class: "big", "med", "small"
    edges = []  # (idx_a, idx_b)

    # Gen 1: first nodes branching from C toward the right
    for ox, oy, src in branch_origins:
        # Direction: away from C center, biased right
        dx = ox - c_optical_x
        dy = oy - c_optical_y
        length = math.hypot(dx, dy) or 1
        dx, dy = dx / length, dy / length

        # Strong rightward bias for open-side origins
        if src == "open":
            dx = dx * 0.3 + 0.9
            dy = dy * 0.5
        else:
            dx = dx * 0.5 + 0.5
            dy = dy * 0.7

        length = math.hypot(dx, dy)
        dx, dy = dx / length, dy / length

        branch_len = r * 0.30
        nx = ox + dx * branch_len
        ny = oy + dy * branch_len

        # Clamp inside circle
        dist = math.hypot(nx - cx, ny - cy)
        if dist > r * 0.85:
            scale = (r * 0.85) / dist
            nx = cx + (nx - cx) * scale
            ny = cy + (ny - cy) * scale

        parent_idx = len(nodes)
        nodes.append((nx, ny, "big"))
        edges.append((-1, parent_idx, ox, oy))  # -1 means origin is on the C

    # Gen 2: secondary branches from gen-1
    gen1_count = len(nodes)
    for i in range(gen1_count):
        nx, ny, _ = nodes[i]
        # Find original direction
        edge = edges[i]
        ox, oy = edge[2], edge[3]
        dx = nx - ox
        dy = ny - oy
        length = math.hypot(dx, dy) or 1
        dx, dy = dx / length, dy / length

        # 1-2 sub-branches
        spreads = [-0.45, 0.45] if i % 2 == 0 else [0.0]
        for spread in spreads:
            sdx = dx * math.cos(spread) - dy * math.sin(spread)
            sdy = dx * math.sin(spread) + dy * math.cos(spread)

            branch_len = r * 0.22
            bx = nx + sdx * branch_len
            by = ny + sdy * branch_len

            dist = math.hypot(bx - cx, by - cy)
            if dist > r * 0.88:
                scale = (r * 0.88) / dist
                bx = cx + (bx - cx) * scale
                by = cy + (by - cy) * scale

            child_idx = len(nodes)
            nodes.append((bx, by, "med"))
            edges.append((i, child_idx, nx, ny))

    # Gen 3: tiny terminal nodes
    gen2_count = len(nodes)
    for i in range(gen1_count, gen2_count):
        nx, ny, _ = nodes[i]
        edge = edges[i]
        px, py = edge[2], edge[3]
        dx = nx - px
        dy = ny - py
        length = math.hypot(dx, dy) or 1
        dx, dy = dx / length, dy / length

        branch_len = r * 0.14
        bx = nx + dx * branch_len
        by = ny + dy * branch_len

        dist = math.hypot(bx - cx, by - cy)
        if dist > r * 0.90:
            scale = (r * 0.90) / dist
            bx = cx + (bx - cx) * scale
            by = cy + (by - cy) * scale

        child_idx = len(nodes)
        nodes.append((bx, by, "small"))
        edges.append((i, child_idx, nx, ny))

    # --- Draw edges (from C and between nodes) ---
    line_w = max(1, size // 180)
    for edge in edges:
        a_idx, b_idx = edge[0], edge[1]
        x2, y2 = nodes[b_idx][0], nodes[b_idx][1]
        if a_idx == -1:
            x1, y1 = edge[2], edge[3]
        else:
            x1, y1 = nodes[a_idx][0], nodes[a_idx][1]

        gen = nodes[b_idx][2]
        if gen == "big":
            color = (200, 220, 255, 60)
        elif gen == "med":
            color = (180, 200, 250, 45)
        else:
            color = (160, 185, 240, 30)
        draw.line([(x1, y1), (x2, y2)], fill=color, width=line_w)

    # Lateral connections between nearby same-gen nodes
    for i in range(len(nodes)):
        for j in range(i + 1, len(nodes)):
            if nodes[i][2] == nodes[j][2]:
                d = math.hypot(nodes[j][0] - nodes[i][0], nodes[j][1] - nodes[i][1])
                if d < r * 0.22:
                    draw.line(
                        [(nodes[i][0], nodes[i][1]), (nodes[j][0], nodes[j][1])],
                        fill=(150, 180, 240, 25),
                        width=max(1, size // 256),
                    )

    # --- Draw nodes (all white) ---
    for x, y, sz in nodes:
        if sz == "big":
            nr = size * 0.024
        elif sz == "med":
            nr = size * 0.016
        else:
            nr = size * 0.010

        # Subtle glow
        glow_r = nr * 2.5
        for g in range(int(glow_r), 0, -1):
            a = int(30 * (1 - g / glow_r))
            draw.ellipse([x - g, y - g, x + g, y + g], fill=(200, 220, 255, a))

        # White node
        draw.ellipse(
            [x - nr, y - nr, x + nr, y + nr],
            fill=(255, 255, 255, 240),
            outline=(220, 230, 255, 255),
            width=max(1, size // 300),
        )

    # --- Draw white C on top ---
    # Dark halo behind
    halo_size = max(5, size // 40)
    for offset in range(halo_size, 0, -1):
        a = int(180 * (1 - offset / halo_size))
        draw.text((tx, ty), "C", fill=(12, 8, 35, a), font=font,
                  stroke_width=offset, stroke_fill=(12, 8, 35, a))

    draw.text((tx, ty), "C", fill=(255, 255, 255, 255), font=font)

    return img


icons_dir = os.path.join(os.path.dirname(__file__), "src-tauri", "icons")
    smile_path = os.path.join(icons_dir, "icon_source.png")
master = Image.open(smile_path).convert("RGBA").resize((512, 512), Image.LANCZOS)

sizes = {
    "icon.png": 512, "32x32.png": 32, "64x64.png": 64,
    "128x128.png": 128, "128x128@2x.png": 256,
    "Square30x30Logo.png": 30, "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71, "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107, "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150, "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310, "StoreLogo.png": 50,
}

for name, sz in sizes.items():
    (master.copy() if sz == 512 else master.resize((sz, sz), Image.LANCZOS)).save(
        os.path.join(icons_dir, name))
    print(f"  {name} ({sz}x{sz})")

ico_sizes = [16, 24, 32, 48, 64, 128, 256]
ico_imgs = [master.resize((s, s), Image.LANCZOS) for s in ico_sizes]
ico_imgs[0].save(os.path.join(icons_dir, "icon.ico"), format="ICO",
                 sizes=[(s, s) for s in ico_sizes], append_images=ico_imgs[1:])
print("  icon.ico")
master.save(os.path.join(icons_dir, "icon.icns"))
print("  icon.icns\nDone!")
