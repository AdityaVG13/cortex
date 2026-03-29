# Native Design Systems Reference: Windows 11 & macOS

Technical reference for building a premium desktop app with native-feeling materials, typography, color, and depth -- targeting WebView/Tauri implementations.

---

## 1. Windows 11 Mica Material

### What Mica Is

Mica is an **opaque, dynamic material** that incorporates the user's theme and desktop wallpaper to paint the background of long-lived windows.
It samples the wallpaper **once** (not continuously) making it highly performant.
As the window moves, Mica dynamically adapts using the wallpaper underneath.

**Key behaviors:**
- Falls back to a solid neutral color when the app window is **inactive** (built-in focus indication).
- Falls back to solid color when transparency is disabled, Battery Saver is on, or on low-end hardware.
- Designed for the **base layer** of apps (bottommost layer, behind all content).

### Mica vs Mica Alt

| Variant | Tinting | Use Case | Fallback Token |
|---------|---------|----------|----------------|
| **Mica** | Subtle wallpaper tint | Standard app backgrounds | `SolidBackgroundFillColorBase` |
| **Mica Alt** | Stronger wallpaper tint | Tabbed title bars, needs contrast between title bar and commanding areas | `SolidBackgroundFillColorBaseAlt` |

Mica Alt is available in Windows App SDK 1.1+ on Windows 11 22000+.

### App Layering with Mica

Windows 11 uses a **two-layer system**:

```
[Base Layer]     = Mica / Mica Alt (app foundation: menus, commands, navigation)
  [Content Layer]  = LayerFillColorDefaultBrush (central experience, cards, content)
```

For Mica Alt with tabbed title bars, a three-layer system:

```
[Base Layer]       = Mica Alt
  [Commanding Layer] = LayerOnMicaBaseAltFillColorDefaultBrush (nav, menu bar)
    [Content Layer]    = LayerFillColorDefaultBrush (main content)
```

Both `LayerFillColorDefaultBrush` and `LayerOnMicaBaseAltFillColorDefaultBrush` are low-opacity solid colors that allow Mica to show through.

---

## 2. Windows 11 Acrylic Material

### What Acrylic Is

Acrylic is a **translucent, blurred material** that creates a frosted-glass effect.
Unlike Mica, it continuously blurs what's behind it.

### Two Acrylic Types

| Type | What Shows Through | Use Case |
|------|-------------------|----------|
| **Background Acrylic** | Desktop wallpaper + other windows | Transient UI: context menus, flyouts, light-dismiss panes |
| **In-app Acrylic** | App content behind the surface | Supporting UI: navigation panes, sidebars that overlap content |

### The Acrylic Recipe (Layer Stack)

Microsoft's acrylic is composed of 5 layers from bottom to top:

```
1. Background content (wallpaper or app content)
2. Gaussian blur
3. Exclusion blend layer (ensures contrast/legibility)
4. Color/tint overlay (personalization)
5. Noise texture (subtle grain, adds tactility)
```

### Acrylic Values for CSS Implementation

Based on the official Fluent Design specification and CSS open-source implementations:

```css
/* === ACRYLIC MATERIAL (CSS approximation) === */

.acrylic {
  /* Core blur effect */
  -webkit-backdrop-filter: blur(30px) saturate(125%);
  backdrop-filter: blur(30px) saturate(125%);

  /* Light theme tint */
  background-color: rgba(247, 247, 247, 0.80);  /* #F7F7F7CC */

  /* Noise texture overlay */
  background-image: url('noise-texture.png');
  background-repeat: repeat;

  /* Border for definition */
  border: 1px solid rgba(229, 229, 229, 1.0);  /* #e5e5e5 */
  border-radius: 8px;

  /* Elevation shadow (Fluent flyout-level) */
  box-shadow:
    0px 25.6px 57.6px rgba(0, 0, 0, 0.14),
    0px 0px 16.4px rgba(0, 0, 0, 0.12);
}

/* Dark theme override */
@media (prefers-color-scheme: dark) {
  .acrylic {
    background-color: rgba(24, 24, 24, 0.66);  /* #181818A8 */
    border-color: rgba(26, 26, 26, 0.10);       /* #1a1a1a1a */
  }
}

/* Fallback for unsupported browsers */
@supports not (backdrop-filter: blur(30px)) {
  .acrylic {
    background-color: rgba(255, 255, 255, 0.90);
  }
  @media (prefers-color-scheme: dark) {
    .acrylic {
      background-color: rgba(32, 32, 32, 0.95);
    }
  }
}
```

**Key values:**
- Blur radius: **30px** (official Fluent spec for transient surfaces)
- Saturation boost: **125%**
- Light tint opacity: **~80%** (`0.80`)
- Dark tint opacity: **~66%** (`0.66`)
- Noise texture: 2% opacity tiled PNG, 128x128 or 256x256

### Generating Noise Texture

Create a tiny noise PNG in code or use this CSS fallback:

```css
.acrylic-noise::after {
  content: '';
  position: absolute;
  inset: 0;
  background-image: url("data:image/svg+xml,%3Csvg viewBox='0 0 256 256' xmlns='http://www.w3.org/2000/svg'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='4' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)' opacity='0.02'/%3E%3C/svg%3E");
  pointer-events: none;
  border-radius: inherit;
}
```

---

## 3. Fluent Design Color System (Dark Theme)

### Complete Neutral Palette (Grey Ramp)

From the Fluent UI source (`packages/tokens/src/global/colors.ts`):

| Token | Hex | Usage Context |
|-------|-----|---------------|
| `grey2` | `#050505` | - |
| `grey4` | `#0a0a0a` | Deepest background (colorNeutralBackground4) |
| `grey6` | `#0f0f0f` | - |
| `grey8` | `#141414` | Dark background (colorNeutralBackground3) |
| `grey10` | `#1a1a1a` | - |
| `grey12` | `#1f1f1f` | Secondary background (colorNeutralBackground2) |
| `grey14` | `#242424` | Primary background (colorNeutralBackground1) |
| `grey16` | `#292929` | - |
| `grey18` | `#2e2e2e` | Subtle pressed state |
| `grey20` | `#333333` | Elevated background (colorNeutralBackground6) |
| `grey22` | `#383838` | Subtle hover state |
| `grey24` | `#3d3d3d` | Stroke 3 (faintest border) |
| `grey26` | `#424242` | - |
| `grey28` | `#474747` | - |
| `grey30` | `#4d4d4d` | - |
| `grey32` | `#525252` | Stroke 2 |
| `grey36` | `#5c5c5c` | Disabled foreground |
| `grey40` | `#666666` | Stroke 1 (primary border) |
| `grey44` | `#707070` | - |
| `grey50` | `#808080` | - |
| `grey60` | `#999999` | Foreground 4 (quaternary text) |
| `grey68` | `#adadad` | Foreground 3 / Stroke Accessible |
| `grey84` | `#d6d6d6` | Foreground 2 (secondary text) |
| `grey94` | `#f0f0f0` | - |
| `grey98` | `#fafafa` | - |

### Dark Theme Token Mappings

From `packages/tokens/src/alias/darkColor.ts`:

```
BACKGROUNDS
  colorNeutralBackground1       = #292929  (grey16)  -- primary surface
  colorNeutralBackground2       = #1f1f1f  (grey12)  -- secondary/recessed
  colorNeutralBackground3       = #141414  (grey8)   -- tertiary/deepest
  colorNeutralBackground4       = #0a0a0a  (grey4)   -- extreme depth
  colorNeutralBackground5       = #000000  (black)   -- absolute black
  colorNeutralBackground6       = #333333  (grey20)  -- elevated/card
  colorNeutralBackgroundInverted = #ffffff  (white)

FOREGROUNDS
  colorNeutralForeground1       = #ffffff  (white)   -- primary text
  colorNeutralForeground2       = #d6d6d6  (grey84)  -- secondary text
  colorNeutralForeground3       = #adadad  (grey68)  -- tertiary text
  colorNeutralForeground4       = #999999  (grey60)  -- quaternary text
  colorNeutralForegroundDisabled = #5c5c5c (grey36)  -- disabled text

STROKES (Borders)
  colorNeutralStroke1           = #666666  (grey40)  -- primary border
  colorNeutralStroke2           = #525252  (grey32)  -- secondary border
  colorNeutralStroke3           = #3d3d3d  (grey24)  -- tertiary border
  colorNeutralStrokeAccessible  = #adadad  (grey68)  -- accessible border

SUBTLE (Hover/Press states)
  colorSubtleBackground         = transparent
  colorSubtleBackgroundHover    = #383838  (grey22)
  colorSubtleBackgroundPressed  = #2e2e2e  (grey18)

SHADOWS
  colorNeutralShadowAmbient     = rgba(0, 0, 0, 0.24)
  colorNeutralShadowKey         = rgba(0, 0, 0, 0.28)
  colorNeutralShadowAmbientLighter = rgba(0, 0, 0, 0.12)
  colorNeutralShadowKeyLighter  = rgba(0, 0, 0, 0.14)
  colorNeutralShadowAmbientDarker = rgba(0, 0, 0, 0.40)
  colorNeutralShadowKeyDarker   = rgba(0, 0, 0, 0.48)
  colorBrandShadowAmbient       = rgba(0, 0, 0, 0.30)
  colorBrandShadowKey           = rgba(0, 0, 0, 0.25)
```

### CSS Custom Properties for Dark Theme

```css
:root[data-theme="dark"] {
  /* Backgrounds */
  --bg-primary: #292929;
  --bg-secondary: #1f1f1f;
  --bg-tertiary: #141414;
  --bg-elevated: #333333;
  --bg-card: #333333;

  /* Foregrounds */
  --fg-primary: #ffffff;
  --fg-secondary: #d6d6d6;
  --fg-tertiary: #adadad;
  --fg-disabled: #5c5c5c;

  /* Strokes */
  --stroke-primary: #666666;
  --stroke-secondary: #525252;
  --stroke-subtle: #3d3d3d;
  --stroke-accessible: #adadad;

  /* Interactive */
  --hover-bg: #383838;
  --pressed-bg: #2e2e2e;
}
```

---

## 4. Elevation & Shadows

### Fluent Elevation Levels

| Element | Elevation | Shadow Blur | Stroke Width |
|---------|-----------|-------------|--------------|
| **Layer** | 1 | Minimal | 1px |
| **Control (rest)** | 2 | 2px | 1px |
| **Control (hover)** | 2 | 2px | 1px |
| **Control (pressed)** | 1 | 1px | 1px |
| **Card** | 8 | 8px | 1px |
| **Tooltip** | 16 | 16px | 1px |
| **Flyout / Menu** | 32 | 32px | 1px |
| **Dialog / Window** | 128 | 128px | 1px |

### CSS Box-Shadow Approximations

Each Fluent shadow combines two layers: an **ambient** (soft, omnidirectional) and a **key** (directional, from above).

```css
/* Shadow tokens for dark theme */
:root[data-theme="dark"] {
  /* shadow2 - Controls at rest */
  --shadow-2: 0px 1px 2px rgba(0, 0, 0, 0.28),
              0px 0px 2px rgba(0, 0, 0, 0.24);

  /* shadow4 - Slight elevation */
  --shadow-4: 0px 2px 4px rgba(0, 0, 0, 0.28),
              0px 0px 2px rgba(0, 0, 0, 0.24);

  /* shadow8 - Cards */
  --shadow-8: 0px 4px 8px rgba(0, 0, 0, 0.28),
              0px 0px 2px rgba(0, 0, 0, 0.24);

  /* shadow16 - Tooltips, dropdowns */
  --shadow-16: 0px 8px 16px rgba(0, 0, 0, 0.28),
               0px 0px 2px rgba(0, 0, 0, 0.24);

  /* shadow28 - Flyouts, popovers */
  --shadow-28: 0px 14px 28px rgba(0, 0, 0, 0.48),
               0px 0px 8px rgba(0, 0, 0, 0.40);

  /* shadow64 - Dialogs */
  --shadow-64: 0px 32px 64px rgba(0, 0, 0, 0.48),
               0px 0px 8px rgba(0, 0, 0, 0.40);
}

/* Light theme shadows use lighter opacities */
:root[data-theme="light"] {
  --shadow-2: 0px 1px 2px rgba(0, 0, 0, 0.14),
              0px 0px 2px rgba(0, 0, 0, 0.12);

  --shadow-4: 0px 2px 4px rgba(0, 0, 0, 0.14),
              0px 0px 2px rgba(0, 0, 0, 0.12);

  --shadow-8: 0px 4px 8px rgba(0, 0, 0, 0.14),
              0px 0px 2px rgba(0, 0, 0, 0.12);

  --shadow-16: 0px 8px 16px rgba(0, 0, 0, 0.14),
               0px 0px 2px rgba(0, 0, 0, 0.12);

  --shadow-28: 0px 14px 28px rgba(0, 0, 0, 0.24),
               0px 0px 8px rgba(0, 0, 0, 0.20);

  --shadow-64: 0px 32px 64px rgba(0, 0, 0, 0.24),
               0px 0px 8px rgba(0, 0, 0, 0.20);
}
```

**Pattern:** The key shadow Y-offset is half the blur radius. Ambient shadow is always `0px 0px Npx`.
For low elevations (2-16), use `AmbientLighter`/`KeyLighter` opacity. For high elevations (28-64), use `AmbientDarker`/`KeyDarker`.

---

## 5. Geometry & Corner Radius

### Windows 11 Corner Radius System

| Token | Value | Usage |
|-------|-------|-------|
| `ControlCornerRadius` | **4px** | Buttons, checkboxes, text inputs, list items, progress bars, sliders |
| `OverlayCornerRadius` | **8px** | Dialogs, flyouts, menus, teaching tips, app windows |
| Tooltip exception | **4px** | Tooltips (small size, uses control radius) |
| Snapped/maximized windows | **0px** | Straight edges when windows touch screen edges |

### Nesting Rule

When elements nest inside rounded containers, reduce inner radius:

```
Inner radius = Outer radius - padding between them
```

Example: An 8px rounded dialog with 4px internal padding has inner content corners of 4px.

```css
.dialog { border-radius: 8px; padding: 16px; }
.dialog .card { border-radius: 4px; }
```

---

## 6. Typography: Segoe UI Variable

### The Windows 11 Type Ramp

All sizes in effective pixels (epx). Optical sizing is automatic via variable font.

| Style | Weight | Size / Line Height | CSS `font-weight` |
|-------|--------|--------------------|--------------------|
| **Caption** | Regular | 12 / 16 epx | 400 |
| **Body** | Regular | 14 / 20 epx | 400 |
| **Body Strong** | Semibold | 14 / 20 epx | 600 |
| **Body Large** | Regular | 18 / 24 epx | 400 |
| **Subtitle** | Semibold | 20 / 28 epx | 600 |
| **Title** | Semibold | 28 / 36 epx | 600 |
| **Title Large** | Semibold | 40 / 52 epx | 600 |
| **Display** | Semibold | 68 / 92 epx | 600 |

### CSS Implementation

```css
/* Font stack with fallbacks */
body {
  font-family: 'Segoe UI Variable', 'Segoe UI', system-ui, -apple-system, sans-serif;
  font-size: 14px;
  line-height: 20px;
  font-weight: 400;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

/* Variable font optical sizing (automatic) */
.text-caption    { font-size: 12px; line-height: 16px; font-weight: 400; }
.text-body       { font-size: 14px; line-height: 20px; font-weight: 400; }
.text-body-strong { font-size: 14px; line-height: 20px; font-weight: 600; }
.text-body-large { font-size: 18px; line-height: 24px; font-weight: 400; }
.text-subtitle   { font-size: 20px; line-height: 28px; font-weight: 600; }
.text-title      { font-size: 28px; line-height: 36px; font-weight: 600; }
.text-title-large { font-size: 40px; line-height: 52px; font-weight: 600; }
.text-display    { font-size: 68px; line-height: 92px; font-weight: 600; }
```

### Segoe UI Variable Axes

| Axis | Range | Behavior |
|------|-------|----------|
| **Weight** (`wght`) | 100 (Thin) to 700 (Bold) | Manual control |
| **Optical Size** (`opsz`) | 8pt to 36pt | **Automatic** -- adjusts counters for legibility at small sizes |

Weight names: Thin (100), Light (300), Semilight (350), Regular (400), Semibold (600), Bold (700).

### Best Practices

- **Regular** weight for body text, **Semibold** for headings
- Left-aligned by default
- Minimum: 14px Semibold or 12px Regular (legibility floor)
- Sentence case for all UI text including titles
- 50-60 characters per line for readability

---

## 7. Spacing & Layout

### Base Unit

Fluent uses a **4px base unit** with a **40x40 epx** alignment grid.

### Common Spacing Values

| Context | Value |
|---------|-------|
| Between buttons | 8 epx |
| Between button and flyout | 8 epx |
| Between control and header | 8 epx |
| Between control and label | 12 epx |
| Between content cards | 12 epx |
| Surface edge to text | 16 epx |
| Controls inside expander indent | 48 epx |
| Title-to-body text spacing | 12 epx |

### Standard vs Compact Sizing

| Mode | Row Height | Use Case |
|------|-----------|----------|
| **Standard** | 40 epx | Touch + pointer, general use |
| **Compact** | 32 epx | Dense information, pointer-primary |

---

## 8. How Microsoft Apps Use Materials

### Windows Terminal
- **Acrylic** in the tab row (toggleable via `"useAcrylicInTabRow": true` in settings.json)
- Custom acrylic opacity per profile for terminal backgrounds
- Mica Alt behind tabs for visual hierarchy

### Microsoft Teams
- **Mica** as the base window material
- Sidebar uses the base layer, chat content uses the content layer
- Cards use `LayerFillColorDefaultBrush` for separation

### Dev Home
- **Mica Alt** for the title bar area (tabbed UI)
- Dashboard widgets use the card pattern with `LayerFillColorDefaultBrush`
- Commanding layer between title bar and content

### Common Patterns
- Title bar always shows the material (extend app into non-client area)
- NavigationView left pane uses content layer pattern
- Cards on Mica create subtle hierarchy without heavy shadows

---

## 9. Apple Design Language (Cross-Reference)

### macOS Materials System

Apple uses **NSVisualEffectView** with semantic material types:

| Material | Use Case |
|----------|----------|
| `windowBackground` | Opaque window backgrounds |
| `sidebar` | Sidebar backgrounds (distinct vibrancy) |
| `titlebar` | Window title bars |
| `menu` | Menu backgrounds |
| `popover` | NSPopover window backgrounds |
| `headerView` | Inline header/footer views |
| `sheet` | Sheet window backgrounds |
| `contentBackground` | Opaque content area backgrounds |
| `hudWindow` | Heads-up display window backgrounds |
| `tooltip` | Tooltip backgrounds |
| `underWindowBackground` | Content beneath the window |
| `underPageBackground` | Behind document pages |

All materials **automatically adapt** to light and dark mode, active and inactive window states.

### Vibrancy

Vibrancy is Apple's equivalent of Fluent's "foreground adapts to material."
It pulls light and color from behind the material to make foreground content (text, symbols, fills) pop.

- Updates in **real-time** as background changes
- Four vibrancy levels for labels (primary, secondary, tertiary, quaternary)
- Quaternary NOT recommended on thin/ultra-thin materials (low contrast)

### Apple's Approach to Dense Data UIs

Activity Monitor and System Preferences/Settings demonstrate Apple's principles:

1. **Generous whitespace** even in dense layouts
2. **Sidebar navigation** with vibrancy providing depth without visual weight
3. **Subtle separators** (hairline 0.5px borders) instead of heavy dividers
4. **Consistent row heights** with ample vertical padding
5. **Typography hierarchy** does the heavy lifting for scannability
6. **Monochrome icons** that match text vibrancy levels

---

## 10. SF Pro Typography System

### macOS Type Ramp

| Style | Size (pt) | Weight | CSS Equivalent |
|-------|-----------|--------|----------------|
| **Large Title** | 26 | Regular (400) | `font-size: 26px; font-weight: 400` |
| **Title 1** | 22 | Regular (400) | `font-size: 22px; font-weight: 400` |
| **Title 2** | 17 | Regular (400) | `font-size: 17px; font-weight: 400` |
| **Title 3** | 15 | Regular (400) | `font-size: 15px; font-weight: 400` |
| **Headline** | 13 | Bold (700) | `font-size: 13px; font-weight: 700` |
| **Body** | 13 | Regular (400) | `font-size: 13px; font-weight: 400` |
| **Callout** | 12 | Regular (400) | `font-size: 12px; font-weight: 400` |
| **Subheadline** | 11 | Regular (400) | `font-size: 11px; font-weight: 400` |
| **Footnote** | 10 | Regular (400) | `font-size: 10px; font-weight: 400` |
| **Caption 1** | 10 | Regular (400) | `font-size: 10px; font-weight: 400` |
| **Caption 2** | 10 | Medium (500) | `font-size: 10px; font-weight: 500` |

### SF Pro Optical Sizing

Like Segoe UI Variable, SF Pro uses optical sizing:
- **SF Pro Text**: Optimized for 19pt and below (looser spacing, heavier strokes, larger counters)
- **SF Pro Display**: Optimized for 20pt and above (tighter spacing, thinner strokes)

In variable font format, this switching is automatic.

### CSS Font Stack (macOS)

```css
body {
  font-family: -apple-system, BlinkMacSystemFont, 'SF Pro', system-ui, sans-serif;
}
```

### Key Differences from Windows

| Aspect | Windows (Segoe UI Variable) | macOS (SF Pro) |
|--------|-----------------------------|----------------|
| Body size | 14px | 13px |
| Heading approach | Semibold weight | Bold weight or larger size |
| Minimum text | 12px Regular | 10px Regular |
| Base unit | 4px | 4px (but denser layouts) |
| Overall density | More spacious | More compact |

---

## 11. Implementing Native-Feel in Tauri / WebView2

### Tauri Configuration for Transparency

**tauri.conf.json:**
```json
{
  "app": {
    "windows": [
      {
        "decorations": false,
        "transparent": true,
        "title": "My App",
        "width": 1200,
        "height": 800
      }
    ]
  }
}
```

On macOS, also set:
```json
{
  "app": {
    "macOSPrivateApi": true
  }
}
```

### Tauri Rust: window-vibrancy Crate

**Cargo.toml:**
```toml
[dependencies]
window-vibrancy = "0.5"
```

**src-tauri/src/main.rs:**
```rust
use tauri::Manager;
use window_vibrancy::{apply_mica, apply_acrylic, apply_vibrancy, NSVisualEffectMaterial};

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();

            // Windows 11: Mica
            #[cfg(target_os = "windows")]
            apply_mica(&window, None)?;

            // Windows 10/11: Acrylic (RGBA tint)
            #[cfg(target_os = "windows")]
            apply_acrylic(&window, Some((0, 0, 0, 10)))?;

            // macOS: Vibrancy
            #[cfg(target_os = "macos")]
            apply_vibrancy(&window, NSVisualEffectMaterial::Sidebar, None, None)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### Platform API Reference

| Function | Platform | Notes |
|----------|----------|-------|
| `apply_mica(window, dark_mode)` | Windows 11 only | `dark_mode`: `Option<bool>` (None = follow system) |
| `apply_acrylic(window, color)` | Windows 10 v1809+ | `color`: `Option<(u8,u8,u8,u8)>` RGBA tint |
| `apply_vibrancy(window, material, state, radius)` | macOS 10.10+ | Use `NSVisualEffectMaterial` enum |
| `clear_mica(window)` | Windows 11 | Remove Mica effect |
| `clear_acrylic(window)` | Windows 10/11 | Remove Acrylic effect |
| `clear_vibrancy(window)` | macOS | Remove vibrancy effect |

### CSS for Transparent WebView

```css
/* CRITICAL: WebView background must be transparent for materials to show */
html, body {
  background: transparent !important;
  height: 100%;
  margin: 0;
  padding: 0;
  overflow: hidden;
}

/* Custom title bar drag region */
.titlebar {
  height: 32px;
  -webkit-app-region: drag;     /* Electron-style */
  user-select: none;
  display: flex;
  align-items: center;
  padding: 0 16px;
}

/* Preserve button interactivity in title bar */
.titlebar button {
  -webkit-app-region: no-drag;
}

/* Tauri-specific drag region */
[data-tauri-drag-region] {
  cursor: default;
}
```

### CSS Acrylic for In-App Elements

When you want acrylic effects on elements *within* the app (e.g., sidebars, modals):

```css
.panel-acrylic {
  position: relative;
  backdrop-filter: blur(20px) saturate(120%);
  -webkit-backdrop-filter: blur(20px) saturate(120%);
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 8px;
}

/* Dark theme card on Mica-like background */
.card {
  background: rgba(255, 255, 255, 0.05);  /* ~5% white = LayerFillColor equivalent */
  border: 1px solid rgba(255, 255, 255, 0.06);
  border-radius: 4px;
  padding: 16px;
}
```

### Performance Optimization for Blur Effects

```css
/* GPU acceleration hints */
.blur-element {
  will-change: backdrop-filter;
  transform: translateZ(0);       /* Force compositing layer */
  contain: strict;                /* Isolate layout recalculations */
}
```

**Critical performance rules:**

1. **Limit blur radius** to 20-30px maximum. Rendering cost scales with radius.
2. **Minimize blur element count.** 1-2 blurred surfaces per view is ideal. 5+ will cause frame drops.
3. **Use `will-change: backdrop-filter`** to pre-promote the element to its own compositing layer.
4. **Use `contain: strict`** on blurred elements to prevent layout thrashing.
5. **Avoid animating blur values.** Change opacity of overlays instead.
6. **Provide solid fallbacks.** On resize/drag, temporarily disable blur:

```css
/* Disable blur during resize for smooth 60fps */
.window-resizing .blur-element {
  backdrop-filter: none;
  background: rgba(30, 30, 30, 0.95);
  transition: none;
}
```

### Known Tauri Issue

There is a **known performance regression** when resizing/dragging windows with vibrancy on Windows 11 build 22621+.
Workaround: disable GPU acceleration with `--disable-gpu` in `additionalBrowserArgs` if the app is not graphics-intensive.

---

## 12. Windows 11 Native Feel Checklist

### Visual Fidelity

- [ ] Mica or Mica Alt as window base layer
- [ ] Custom title bar extending into non-client area
- [ ] 8px rounded corners on window and overlays
- [ ] 4px rounded corners on controls
- [ ] Subtle 1px borders (`colorNeutralStroke3` / `#3d3d3d` dark)
- [ ] Dual-layer shadows (ambient + key) on elevated surfaces
- [ ] Noise texture on acrylic surfaces

### Typography

- [ ] Segoe UI Variable as primary font
- [ ] Body text at 14px Regular
- [ ] Headings at Semibold weight
- [ ] Minimum 12px for any text
- [ ] Sentence case everywhere

### Color & Theme

- [ ] Respect `prefers-color-scheme` media query
- [ ] Dark backgrounds: `#292929` primary, `#1f1f1f` secondary
- [ ] White text on dark: `#ffffff` primary, `#d6d6d6` secondary
- [ ] Accent color from system (via Windows APIs)
- [ ] Inactive window state differentiation

### Interaction

- [ ] Hover state: `colorSubtleBackgroundHover` (`#383838`)
- [ ] Pressed state: `colorSubtleBackgroundPressed` (`#2e2e2e`)
- [ ] Focus indicators visible (accessible stroke `#adadad`)
- [ ] Smooth transitions (150-250ms, ease-out curves)

### Spacing

- [ ] 4px base unit grid alignment
- [ ] 8px between related controls
- [ ] 12px between content sections
- [ ] 16px edge margins
- [ ] 32px or 40px row heights

---

## 13. Open-Source Implementations

### CSS Libraries

| Project | Description | URL |
|---------|-------------|-----|
| **acrylic-mica-css** | CSS Mica & Acrylic recreation | [github.com/yell0wsuit/acrylic-mica-css](https://github.com/yell0wsuit/acrylic-mica-css) |
| **fluent-design-acrylic** | CSS acrylic with blur + grain | [github.com/Smilebags/fluent-design-acrylic](https://github.com/Smilebags/fluent-design-acrylic) |
| **Fluent UI React v9** | Official Microsoft components | [github.com/microsoft/fluentui](https://github.com/microsoft/fluentui) |
| **Fluent UI Web Components** | Framework-agnostic web components | [github.com/microsoft/fluentui/tree/master/packages/web-components](https://github.com/microsoft/fluentui) |

### Tauri Libraries

| Crate | Description | URL |
|-------|-------------|-----|
| **window-vibrancy** | Mica, Acrylic, macOS vibrancy | [github.com/tauri-apps/window-vibrancy](https://github.com/tauri-apps/window-vibrancy) |

---

## 14. Quick-Start CSS Variables (Complete Dark Theme)

Copy-paste foundation for a Fluent-inspired dark theme:

```css
:root {
  /* === COLORS === */
  --color-bg-base: #1f1f1f;          /* Window background (Mica shows through) */
  --color-bg-primary: #292929;        /* Primary surface */
  --color-bg-secondary: #1f1f1f;      /* Recessed areas */
  --color-bg-tertiary: #141414;       /* Deepest background */
  --color-bg-elevated: #333333;       /* Cards, elevated panels */
  --color-bg-layer: rgba(255, 255, 255, 0.05);  /* Layer on Mica */
  --color-bg-layer-alt: rgba(255, 255, 255, 0.03); /* Layer on Mica Alt */

  --color-fg-primary: #ffffff;
  --color-fg-secondary: #d6d6d6;
  --color-fg-tertiary: #adadad;
  --color-fg-disabled: #5c5c5c;

  --color-stroke-default: #666666;
  --color-stroke-subtle: #525252;
  --color-stroke-divider: #3d3d3d;
  --color-stroke-focus: #adadad;

  --color-hover: #383838;
  --color-pressed: #2e2e2e;

  /* === ACRYLIC === */
  --acrylic-blur: 30px;
  --acrylic-saturation: 125%;
  --acrylic-tint-dark: rgba(24, 24, 24, 0.66);
  --acrylic-tint-light: rgba(247, 247, 247, 0.80);

  /* === SHADOWS === */
  --shadow-card: 0px 4px 8px rgba(0, 0, 0, 0.28), 0px 0px 2px rgba(0, 0, 0, 0.24);
  --shadow-flyout: 0px 14px 28px rgba(0, 0, 0, 0.48), 0px 0px 8px rgba(0, 0, 0, 0.40);
  --shadow-dialog: 0px 32px 64px rgba(0, 0, 0, 0.48), 0px 0px 8px rgba(0, 0, 0, 0.40);

  /* === GEOMETRY === */
  --radius-control: 4px;
  --radius-overlay: 8px;
  --radius-none: 0px;

  /* === SPACING === */
  --space-xxs: 2px;
  --space-xs: 4px;
  --space-sm: 8px;
  --space-md: 12px;
  --space-lg: 16px;
  --space-xl: 24px;
  --space-xxl: 32px;

  /* === TYPOGRAPHY === */
  --font-family: 'Segoe UI Variable', 'Segoe UI', system-ui, -apple-system, sans-serif;
  --font-size-caption: 12px;
  --font-size-body: 14px;
  --font-size-body-large: 18px;
  --font-size-subtitle: 20px;
  --font-size-title: 28px;
  --font-size-title-large: 40px;
  --font-size-display: 68px;

  --line-height-caption: 16px;
  --line-height-body: 20px;
  --line-height-body-large: 24px;
  --line-height-subtitle: 28px;
  --line-height-title: 36px;
  --line-height-title-large: 52px;
  --line-height-display: 92px;

  --font-weight-regular: 400;
  --font-weight-semibold: 600;

  /* === TRANSITIONS === */
  --transition-fast: 100ms ease-out;
  --transition-normal: 200ms ease-out;
  --transition-slow: 300ms ease-out;
}
```

---

## Sources

- [Mica Material - Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/design/style/mica)
- [Acrylic Material - Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
- [Typography in Windows - Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography)
- [Color in Windows - Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/color)
- [Layering and Elevation - Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/layering)
- [Geometry in Windows 11 - Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry)
- [Shadows in Windows Apps - Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/design/layout/depth-shadow)
- [Content Layout and Spacing - Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/design/style/spacing)
- [Rounded Corners - Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/design/style/rounded-corner)
- [Fluent 2 Design System - Color](https://fluent2.microsoft.design/color)
- [Fluent 2 Design System - Elevation](https://fluent2.microsoft.design/elevation)
- [Fluent 2 Design System - Typography](https://fluent2.microsoft.design/typography)
- [Fluent 2 Design System - Design Tokens](https://fluent2.microsoft.design/design-tokens)
- [Fluent UI React v9 Token Source - darkColor.ts](https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/darkColor.ts)
- [Fluent UI Grey Palette Source - colors.ts](https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/colors.ts)
- [window-vibrancy Crate - Tauri](https://github.com/tauri-apps/window-vibrancy)
- [Tauri Window Customization](https://v2.tauri.app/learn/window-customization/)
- [Acrylic CSS Implementation - yell0wsuit](https://github.com/yell0wsuit/acrylic-mica-css)
- [DIY Web Acrylic - Microsoft Design (Medium)](https://medium.com/microsoft-design/diy-a-web-version-the-fluent-design-systems-acrylic-material-fe2eac2a40bb)
- [Acrylic Window Effect with Tauri - DEV Community](https://dev.to/waradu/acrylic-window-effect-with-tauri-1078)
- [Apple HIG - Materials](https://developer.apple.com/design/human-interface-guidelines/materials)
- [Apple HIG - Typography](https://developer.apple.com/design/human-interface-guidelines/typography)
- [NSVisualEffectView - Apple Developer](https://developer.apple.com/documentation/appkit/nsvisualeffectview)
- [iOS Default Font Sizes - zacwest](https://gist.github.com/zacwest/916d31da5d03405809c4)
