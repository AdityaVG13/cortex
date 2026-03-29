# UI/UX Design Patterns Reference
## Arc Browser, Linear, and Raycast

A comprehensive design blueprint compiled from official documentation, design blogs, community resources, open-source projects, and reverse-engineered specifications.

---

## Table of Contents

1. [Arc Browser (The Browser Company)](#1-arc-browser)
2. [Linear (linear.app)](#2-linear)
3. [Raycast (raycast.com)](#3-raycast)
4. [Cross-Cutting Patterns](#4-cross-cutting-patterns)
5. [Sources & Resources](#5-sources--resources)

---

## 1. Arc Browser

### Design Philosophy

Arc's design is rooted in **"mental calm"** -- clean, minimal, and frictionless interfaces that prioritize clarity over decoration. The Browser Company draws inspiration from film, video games, and visual art (notably Robert Irwin's scrim installations). Their approach: every interface element should serve the user, not impress them.

Key principles:
- **Prototype-heavy, no formal PRDs** -- everyone is empowered to make demos
- **Scrappy by design** -- no formal PMs; cross-functional ownership
- **Storytelling** -- every feature ships with a narrative
- **Immersive browsing** -- the browser chrome recedes; the web page is the star

### 1.1 Blur & Translucency Effects

Arc uses macOS-native vibrancy and glassmorphism extensively:

**Implementation approach:**
- Built in **Swift/SwiftUI** on macOS, leveraging `NSVisualEffectView` for native vibrancy
- The sidebar has a translucent, frosted-glass appearance that lets the desktop/content bleed through
- The entire browser frame acts as a "scrim" -- a subtly lit border around content (inspired by artist Robert Irwin)

**CSS/Web equivalent techniques:**
```css
/* Glassmorphism for sidebar-like panels */
.sidebar {
  background: rgba(30, 30, 30, 0.72);
  backdrop-filter: blur(12px) saturate(180%);
  -webkit-backdrop-filter: blur(12px) saturate(180%);
  border-right: 1px solid rgba(255, 255, 255, 0.08);
}

/* Performance-optimal blur range: 8-15px */
/* Heavy blur (20px+) impacts GPU rendering */
```

**Key observations:**
- Blur intensity is moderate (not extreme frosted glass, but enough for depth)
- Saturation boost (~180%) keeps colors vibrant behind the blur
- The translucency adapts to each Space's theme color, so the sidebar takes on a warm/cool cast
- On Windows (non-native), Arc uses a simulated translucency with semi-transparent backgrounds

### 1.2 Sidebar Navigation Design

Arc's sidebar is its defining UI innovation -- a vertical tab manager that replaces the traditional horizontal tab strip.

**Structural hierarchy (top to bottom):**

| Section | Behavior | Position |
|---------|----------|----------|
| **Favorites** | Icons only, persist across all Spaces | Very top of sidebar, above Space titles |
| **Space Selector** | Horizontal row of Space icons, two-finger swipe to switch | Below favorites |
| **Pinned Tabs** | Bookmarks that persist in the current Space; can be grouped into folders | Upper half, above the divider line |
| **Divider Line** | Horizontal separator | Mid-sidebar |
| **Unpinned Tabs** | Active/recent tabs, auto-archived after 12 hours by default | Lower half, below the divider |

**Dimensions (from ArcWTF Firefox recreation and Figma files):**
- **Expanded sidebar width:** ~300px
- **Collapsed charm bar:** ~40px wide (icon-only rail)
- **Tab item height:** ~32px (compact, icon + truncated title)
- **Favicon size:** 16px (standard), with 20px leading icons for pinned sites
- **Sidebar hover zone:** Extends slightly beyond the sidebar edge for easy reveal
- **Default sidebar opacity when collapsed:** Dimmed; `uc.tweak.af.no-dimming` forces 100%

**Information density approach:**
- Vertical layout = more tabs visible at once vs. horizontal tabs
- Favorites use icon-only display (no text labels) -- maximum density
- Pinned tabs show truncated titles; folders collapse groups
- Unpinned tabs auto-archive, so the list never grows unbounded
- Split View allows side-by-side browsing without leaving the single window
- Each Space is a self-contained environment (its own pins, theme, icon)

### 1.3 Animation & Transition Patterns

Arc is known for fluid, spring-based animations built in SwiftUI:

**SwiftUI animation values (from reproduction studies):**
```swift
// Search bar expand animation
.spring(duration: 0.09)    // Quick spring for initial pop
.easeOut(duration: 0.38)   // Smooth settle
.easeInOut(duration: 0.08) // Micro-adjustment
.linear(duration: 0.01)    // Snap finish

// Tab switching
.animation(.spring(), value: currentTab)

// Sidebar reveal/hide
// Uses matchedGeometryEffect with @Namespace for seamless transitions
```

**Web CSS equivalents:**
```css
/* Sidebar slide-in */
.sidebar {
  transition: transform 0.3s cubic-bezier(0.2, 0.9, 0.3, 1.0),
              opacity 0.2s ease-out;
}

/* Tab hover */
.tab-item {
  transition: background-color 0.15s ease,
              transform 0.2s cubic-bezier(0.34, 1.56, 0.64, 1);
}

/* Space switching -- crossfade with scale */
.space-content {
  transition: opacity 0.25s ease-in-out,
              transform 0.3s cubic-bezier(0.2, 0.9, 0.3, 1.0);
}
```

**Key animation characteristics:**
- **Spring physics** -- almost everything uses spring curves, not linear/ease
- **Micro-interactions everywhere** -- hover states, tab close, sidebar reveal
- **Playful but purposeful** -- animations are fast (under 400ms) and never block interaction
- **Gesture-driven** -- two-finger swipe for Spaces feels like native iOS page curl

### 1.4 Color System

Arc's color system is **per-Space theming** -- each Space has its own color that tints the entire browser chrome.

**CSS Custom Properties injected by Arc into every page:**
```css
:root {
  /* Core palette */
  --arc-palette-background: #F4EBE5FF;        /* Page-area tint */
  --arc-palette-backgroundExtra: #FEFDFCFF;   /* Elevated surfaces */
  --arc-palette-foregroundPrimary: #EF8C62FF;  /* Primary accent */
  --arc-palette-foregroundSecondary: #EF8C62FF;/* Secondary accent */
  --arc-palette-foregroundTertiary: #FCEBE4FF; /* Tertiary/muted */
  --arc-palette-title: #2E1000FF;              /* Title text */
  --arc-palette-subtitle: #D4AD97FF;           /* Subtitle text */
  --arc-palette-hover: #E9D6CBFF;              /* Hover state */
  --arc-palette-focus: #CA997EFF;              /* Focus ring */
  --arc-palette-cutoutColor: #FCEBE4FF;        /* Cutout overlays */
  --arc-palette-maxContrastColor: #993810FF;   /* Maximum contrast */
  --arc-palette-minContrastColor: #FCEBE4FF;   /* Minimum contrast */

  /* Gradient system */
  --arc-background-simple-color: #EF8C62FF;
  --arc-background-gradient-color0: /* varies */;
  --arc-background-gradient-color1: /* varies */;
  --arc-background-gradient-overlay-color0: /* varies */;
}
```

**Important behavior:**
- These variables are injected **after page load** -- `var(--arc-palette-background)` is momentarily undefined during initial render
- Detect Arc Browser: check if `--arc-palette-background` has a value
- Users customize Space colors through a color picker; the system generates the full palette from a single base hue
- Both dark and light palette variants are provided

**What makes it feel "premium":**
- The gently lit frame around the entire browser -- subtle gradient border
- Soft gradients throughout (not flat colors)
- Purposeful use of whitespace
- The sidebar tint shifts warmth based on the Space color
- Everything feels "alive" because the color palette is dynamic, not static

### 1.5 Typography

Arc uses the system sans-serif stack for maximum native feel:

- **UI Text:** SF Pro (macOS) / Segoe UI (Windows)
- **Tab titles:** 13px, regular weight, single-line truncation with ellipsis
- **Section headers (Pinned, Today, etc.):** 11px, semibold, uppercase tracking
- **URL bar:** Larger font (~15-16px), medium weight
- **Design goal:** Typography should feel native to the OS, not branded

### 1.6 Open-Source & Community Resources

| Resource | URL |
|----------|-----|
| **ArcWTF** (Firefox CSS recreation) | [github.com/KiKaraage/ArcWTF](https://github.com/KiKaraage/ArcWTF) |
| **Arc Figma UI Kit** (mockup + components) | [figma.com/community/file/1206735913962953604](https://www.figma.com/community/file/1206735913962953604) |
| **Arc Figma Interface** (editable) | [figma.com/community/file/1228728710215940920](https://www.figma.com/community/file/1228728710215940920) |
| **EdgyArc-fr** (Firefox + Sidebery CSS) | [github.com/artsyfriedchicken/EdgyArc-fr](https://github.com/artsyfriedchicken/EdgyArc-fr) |
| **Arc CSS Theme Variables** (blog post) | [ginger.wtf/posts/creating-a-theme-using-arc/](https://ginger.wtf/posts/creating-a-theme-using-arc/) |

---

## 2. Linear

### Design Philosophy

Linear is the gold standard for "luxurious tool" design. CEO Karri Saarinen (formerly Airbnb design system lead) established a culture where **craft is non-negotiable** and intuition beats A/B testing.

**Core principles (Karri Saarinen's rules):**
1. Quality is a feature -- never ship anything that feels unfinished
2. Design and engineering are not separate disciplines -- some designers code, some engineers design
3. No A/B testing -- trust your intuition after building deep craft
4. Hire people who think about the product and business broadly
5. Connect the team directly with users (shared Slack channels)
6. Set a standards bar, then give freedom to meet it
7. Early design needs freedom; later design needs reality
8. Prioritize thinking phase before execution
9. Speed is a feature, not just a metric
10. The product should feel as fast and efficient as the tool itself

### 2.1 Typography

**Font stack:**
- **Headings:** Inter Display (variable) -- more optical refinement for larger sizes
- **Body/UI:** Inter (variable) -- the workhorse for all non-heading text
- **Monospace (code blocks, IDs):** JetBrains Mono or SF Mono

**Type scale (estimated from UI):**
```css
/* Linear-style type scale */
--font-heading-xl: 600 24px/1.2 'Inter Display', sans-serif;
--font-heading-lg: 600 20px/1.3 'Inter Display', sans-serif;
--font-heading-md: 600 16px/1.4 'Inter Display', sans-serif;
--font-heading-sm: 600 14px/1.4 'Inter Display', sans-serif;
--font-body:      400 14px/1.5 'Inter', sans-serif;
--font-body-sm:   400 13px/1.5 'Inter', sans-serif;
--font-caption:   500 11px/1.4 'Inter', sans-serif;
--font-mono:      400 13px/1.5 'JetBrains Mono', monospace;
```

**Key typography decisions:**
- Inter Display for headings adds "expression" while maintaining readability
- Regular Inter for body text keeps information scannable
- Sans-serif is chosen deliberately for dark mode (cleaner rendering on dark backgrounds)
- Tight line-heights for density; generous for long-form content

### 2.2 Color Palette

**Brand colors (from Mobbin):**

| Name | Hex | RGB | Usage |
|------|-----|-----|-------|
| **Indigo** (signature) | `#5E6AD2` | 94, 106, 210 | Primary accent, brand, CTAs |
| **Woodsmoke** | `#191A1F` | 25, 26, 31 | Dark backgrounds |
| **Oslo Gray** | `#6B6F76` | 107, 111, 118 | Secondary text, borders |
| **Black Haze** | `#F2F3F5` | 242, 243, 245 | Light mode background |
| **White** | `#FBFBFB` | 251, 251, 251 | Cards, elevated surfaces (light) |

**Dark theme surface elevation system (estimated from VS Code theme + inspection):**

| Surface | Hex | Usage |
|---------|-----|-------|
| Base background | `#0D0E12` | Deepest background layer |
| Sidebar/Panel | `#131416` | Navigation sidebar |
| Card/Elevated | `#1B1C22` | Issue cards, modals |
| Hover state | `#1F2028` | Interactive hover |
| Border | `#26272C` | Subtle dividers |
| Active/Selected | `#2A2B33` | Selected items |
| Text primary | `#EEEEEE` | Main content text |
| Text secondary | `#8A8F98` | Metadata, timestamps |
| Text muted | `#505258` | Disabled, placeholder |

**Color system architecture:**
- Built on **LCH color space** (not HSL) for perceptual uniformity
- Only **three inputs** generate an entire theme: base color, accent color, contrast level
- LCH ensures a red and yellow at lightness 50 appear equally light to the human eye
- The system generates surface elevations, borders, and complementary shades from those three inputs
- Custom themes only require setting background, text, and accent -- everything else is derived

### 2.3 Keyboard-First Design

Linear is arguably the most keyboard-optimized SaaS product in existence:

**Navigation shortcuts (G-prefix pattern):**
| Shortcut | Action |
|----------|--------|
| `G` then `I` | Go to Inbox |
| `G` then `M` | Go to My Issues |
| `G` then `T` | Go to Triage |
| `G` then `A` | Go to Active Issues |
| `G` then `B` | Go to Backlog |
| `G` then `C` | Go to Cycles |
| `G` then `P` | Go to Projects |
| `G` then `S` | Go to Settings |

**Command palette (Cmd+K):**
- Powered by the same `cmdk` library by Paco Coursey that Raycast and Vercel use
- Fuzzy search across all sections
- Recent actions surfaced first
- Nested command groups (e.g., "Create" > "Issue", "Project", "Document")

**Design patterns for keyboard-first UX:**
- Every action has a discoverable shortcut shown in context menus and tooltips
- Keyboard shortcut hints appear as `kbd` badges: light background, subtle border, monospace
- `?` key opens a full shortcut reference overlay
- Actions are organized by frequency: most common = single key, less common = chord

### 2.4 Gradients & Depth Effects

**The Linear gradient signature:**
```css
/* Angular gradient (the sphere/logo style) */
background: conic-gradient(
  from 180deg,
  #08AEEA 0%,
  #2AF598 25%,
  #B5FFFC 35%,
  #FF5ACD 60%,
  #5E6AD2 80%,
  #08AEEA 100%
);
filter: blur(40px);
opacity: 0.6;
```

**Depth and elevation:**
```css
/* Card elevation */
.card {
  background: var(--surface-elevated);
  border: 1px solid var(--border-default);
  border-radius: 8px;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1),
              0 0 0 1px rgba(0, 0, 0, 0.05);
}

/* Modal/dialog elevation */
.modal {
  background: var(--surface-overlay);
  border: 1px solid var(--border-default);
  border-radius: 12px;
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4),
              0 0 0 1px rgba(0, 0, 0, 0.1);
}

/* Subtle glow on interactive elements */
.button-primary:hover {
  box-shadow: 0 0 0 1px var(--accent),
              0 0 24px rgba(94, 106, 210, 0.3);
}
```

**Key depth techniques:**
- Very subtle box-shadows (1-2px blur) for cards
- Dramatic shadows for overlays/modals (16-48px blur)
- Single-pixel border + shadow combo for "lifted" effect
- Accent glow on hover states (the indigo radiates outward)
- Background gradient blurs as decorative elements (the "streamers")

### 2.5 Dark Mode Implementation

Linear's dark mode is considered exceptional because:

**Contrast management:**
- Text and neutral icons are made **lighter** in dark mode for improved contrast
- Content contrast ratios target 15.8:1 for primary text on dark surfaces
- Chrome (UI framework color, in their case blue) usage is intentionally limited in calculations to keep things neutral

**Surface hierarchy:**
- Background surfaces increase in lightness for higher elevation (opposite of light mode)
- Panels < Cards < Modals < Tooltips (each slightly lighter)
- This matches Material Design's dark theme elevation model

**LCH-based theme generation:**
```
Inputs: base_color, accent_color, contrast_level (30-100)

For each surface level:
  lightness = base_lightness + (elevation_step * contrast_modifier)
  chroma = base_chroma * 0.3  // keep surfaces near-neutral
  hue = base_hue

Border colors: slight chroma increase from adjacent surface
Text colors: high lightness, low chroma for readability
Accent: full chroma preservation for interactive elements
```

### 2.6 Card/List Transitions and Animations

**Micro-interactions:**
- Issue status changes trigger a subtle slide + fade
- Drag-and-drop uses spring physics for the floating card
- List reordering animates items smoothly into new positions
- Priority/status icon changes include a brief color pulse

**Animation timing (estimated):**
```css
/* Quick state changes */
--transition-fast: 100ms ease;
/* Standard interactions */
--transition-default: 200ms cubic-bezier(0.25, 0.1, 0.25, 1);
/* Layout shifts */
--transition-layout: 300ms cubic-bezier(0.2, 0.9, 0.3, 1.0);
/* Overlays appearing */
--transition-overlay: 150ms ease-out;
```

### 2.7 2026 UI Refresh ("A Calmer Interface")

Linear's March 2026 refresh focused on **reducing visual noise**:

**Sidebar:**
- Navigation sidebar is now **dimmer** -- it recedes once you've reached your destination
- Previously it remained equally prominent as the content area

**Tabs:**
- More compact (no longer span full width)
- Rounded corners on tab containers
- Smaller icon and text sizing

**Icons:**
- Redrawn and resized across the entire app
- Consistent stroke width and optical sizing

**Navigation density:**
- Headers, navigation, and view controls are now **consistent across projects, issues, reviews, and documents**
- Adjusted to increase hierarchy and density of navigation elements

**Design principle behind the refresh:**
> "In a product as information-dense as Linear, not every element of the interface should carry equal visual weight. Parts central to the user's task should stay in focus; ones that support orientation and navigation should recede."

### 2.8 Open-Source & Community Resources

| Resource | URL |
|----------|-----|
| **Linear Style** (70+ community themes) | [linear.style](https://linear.style/) |
| **Linear VS Code Theme** (color reference) | [github.com/pabueco/linear-vscode-theme](https://github.com/pabueco/linear-vscode-theme) |
| **Linear Figma Design System** | [figma.com/community/file/1222872653732371433](https://www.figma.com/community/file/1222872653732371433) |
| **Linear UI Free Kit (recreated)** | [figma.com/community/file/1279162640816574368](https://www.figma.com/community/file/1279162640816574368) |
| **cmdk** (command palette library) | [github.com/pacocoursey/cmdk](https://cmdk.paco.me/) |
| **Catppuccin for Linear** | [github.com/catppuccin/linear](https://github.com/catppuccin/linear) |

---

## 3. Raycast

### Design Philosophy

Raycast is built on three principles: **Fast, Simple, Delightful.** Founded by Thomas Paul Mann and Petr Nikolaev (both ex-Facebook), the product obsesses over keyboard-driven productivity with native macOS polish.

Core beliefs:
- A better way of using computers -- simpler, faster, more delightful
- Every feature should be discoverable through the command palette
- Extensions must look and feel like first-party features
- Native macOS integration (not Electron) for performance and feel
- The UI is 99% text, but it does enormous amounts of functionality

### 3.1 Command Palette Design

The Raycast window is a floating panel that appears on keyboard shortcut and vanishes on blur.

**Window structure (top to bottom):**

| Zone | Content | Height |
|------|---------|--------|
| **Search Bar** | Text input + extension icon | ~48px |
| **Result List** | Scrollable items with icons, titles, subtitles, accessories | ~400px max |
| **Action Bar** | Navigation title (left), available actions + shortcuts (right) | ~36px |

**Key design decisions:**
- **Search bar is oversized** relative to results -- reflects its importance and grabs attention
- **Leading icons are large** -- quicker to scan visually
- **Action bar at bottom** shows contextual keyboard shortcuts -- users learn shortcuts organically
- **Compact Mode** (optional): opens with just the search bar, expands as you type

**Compact Mode behavior:**
1. Activation shows only the search input (minimal footprint)
2. As you type, results appear below with a smooth expand animation
3. Blends all secondary elements for a minimal appearance
4. Focuses the user on the search interaction

**Result item anatomy:**
```
[Icon 20px] [Title - primary text] [Subtitle - secondary text] [Accessories - right aligned]
            [Detail text - optional]                            [Keyboard shortcut badge]
```

**Web implementation reference (cmdk-based):**
```css
/* Raycast-style command palette */
.command-palette {
  max-width: 750px;
  width: 100%;
  border-radius: 12px;
  overflow: hidden;
  background: var(--bg-primary);
  box-shadow:
    0 0 0 1px rgba(255, 255, 255, 0.05),
    0 16px 70px rgba(0, 0, 0, 0.5),
    0 2px 8px rgba(0, 0, 0, 0.2);
}

.search-input {
  height: 48px;
  padding: 0 16px;
  font-size: 15px;
  font-weight: 400;
  border-bottom: 1px solid var(--border-subtle);
  background: transparent;
}

.result-item {
  height: 44px;
  padding: 0 16px;
  display: flex;
  align-items: center;
  gap: 12px;
  border-radius: 8px;
  margin: 0 8px;
  cursor: default;
}

.result-item[data-selected="true"] {
  background: var(--bg-selected);
}

.action-bar {
  height: 36px;
  padding: 0 16px;
  border-top: 1px solid var(--border-subtle);
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  color: var(--text-secondary);
}

.kbd-badge {
  display: inline-flex;
  align-items: center;
  padding: 2px 6px;
  font-size: 11px;
  font-family: 'Inter', sans-serif;
  font-weight: 500;
  border-radius: 4px;
  background: var(--bg-kbd);
  border: 1px solid var(--border-kbd);
  color: var(--text-secondary);
}
```

### 3.2 Extension/Plugin UI Consistency

Raycast enforces design consistency through its extension API:

**Available view types:**
- **List** -- vertical scrollable list with optional detail pane (most common)
- **Grid** -- icon/thumbnail grid layout
- **Detail** -- full markdown rendering with metadata
- **Form** -- input fields, dropdowns, toggles for creating content

**API-enforced patterns:**
- All extensions use the same `List.Item`, `Grid.Item`, `Detail`, `Form` components
- Colors adapt automatically to the active theme (light/dark)
- Icons must use Raycast's built-in icon set or SF Symbols
- Accessories (right-side metadata) follow a strict type system: text, icon, date, tag
- Action panels share the same keyboard shortcut conventions

**Result: every extension feels like a first-party feature** because the building blocks are identical.

### 3.3 Typography & Icon System

**Typography:**
- **Primary font:** Inter 4.0 (upgraded from SF Pro)
- **Search input:** ~15px, regular weight
- **List item title:** 14px, medium weight (500)
- **List item subtitle:** 13px, regular weight (400), secondary color
- **Action bar text:** 12px, medium weight
- **Keyboard badges:** 11px, medium weight, monospace-like spacing

**Why Inter 4.0:**
- Specifically designed for UI use at small sizes
- Variable font with fine-grained weight control
- Excellent cross-platform rendering (macOS + future platforms)
- Better tabular figures for aligned numerical data

**Icon system (redesigned by James McDonald):**
- **Style:** Outline-only (no filled variants in standard use)
- **Stroke width:** Bold/consistent across all icons (thicker than typical)
- **Corner radii:** Uniform across the set
- **Design goal:** "Fresh and timeless" -- simple enough to scan at 16px

**Built-in icon categories** (from Raycast API):
`AddPerson`, `Airplane`, `Alarm`, `AppWindow`, `ArrowLeft`, `Binoculars`, `Bookmark`, `Bug`, `Calendar`, `Camera`, `Checkmark`, `Circle`, `Clipboard`, `Clock`, `Cloud`, `Code`, `Cog`, `CommandSymbol`, `Compass`, `Desktop`, `Document`, `Download`, `EditShape`, `Envelope`, `ExclamationMark`, `Eye`, `EyeSlash`, `Finder`, `Folder`, `Forward`, `GameController`, `Gear`, `Gift`, `Globe`, `Hammer`, `HardDrive`, `Heart`, `House`, `Image`, `Key`, `Keyboard`, `Layers`, `LevelMeter`, `Link`, `List`, `Lock`, `MagnifyingGlass`, `Map`, `Message`, `Microphone`, `Minus`, `Mobile`, `Monitor`, `Moon`, `Music`, `Network`, `Paperclip`, `Pencil`, `Person`, `Phone`, `Pin`, `Play`, `Plug`, `Plus`, `Power`, `Print`, `QuestionMark`, `Redo`, `Repeat`, `RotateAntiClockwise`, `RotateClockwise`, `Shield`, `Shuffle`, `Sidebar`, `Signal`, `StackedBars`, `Star`, `Stop`, `Sun`, `Tag`, `Terminal`, `Text`, `Trash`, `Tray`, `Trophy`, `TwoPeople`, `Undo`, `Upload`, `Video`, `Wand`, `Warning`, `Wifi`, `Window`, `Wrench`, `XMark`

**Color tinting:**
- Icons accept color tinting via the `Color` API
- Built-in semantic colors: `Blue`, `Green`, `Magenta`, `Orange`, `Purple`, `Red`, `Yellow`
- Dynamic colors: `PrimaryText`, `SecondaryText` (adapt to theme)
- Custom hex colors supported: `#RRGGBB` format

### 3.4 Floating Window Aesthetic

**macOS-native panel behavior:**
- Appears as a floating panel (likely `NSPanel` under the hood)
- Does not activate the owning application when interacting
- Dismisses on click outside (blur event) or Escape key
- Centers on the active screen
- Vibrancy/translucency on the background (system-level, not CSS)

**Visual characteristics:**
- Large corner radius (~12px) -- feels like a macOS sheet
- Deep shadow creates floating/levitating effect
- Subtle 1px border (rgba white at low opacity) for definition against dark backgrounds
- Background uses macOS vibrancy material -- semi-translucent with blur

**Design technique for web reproduction:**
```css
/* Raycast-style floating window */
.floating-panel {
  position: fixed;
  top: 20%;
  left: 50%;
  transform: translateX(-50%);
  width: min(750px, 90vw);
  max-height: 60vh;

  background: rgba(28, 28, 30, 0.88);
  backdrop-filter: blur(24px) saturate(150%);
  border-radius: 12px;
  border: 1px solid rgba(255, 255, 255, 0.06);

  box-shadow:
    0 0 0 0.5px rgba(0, 0, 0, 0.3),
    0 4px 16px rgba(0, 0, 0, 0.3),
    0 24px 80px rgba(0, 0, 0, 0.5);

  /* Appear animation */
  animation: panel-appear 0.15s ease-out;
}

@keyframes panel-appear {
  from {
    opacity: 0;
    transform: translateX(-50%) scale(0.96);
  }
  to {
    opacity: 1;
    transform: translateX(-50%) scale(1);
  }
}
```

### 3.5 Theming System

Raycast supports extensive theming:

**Default dark theme (estimated values):**

| Token | Estimated Hex | Usage |
|-------|---------------|-------|
| Background primary | `#1C1C1E` | Main window background |
| Background secondary | `#2C2C2E` | Selected item, hover |
| Background tertiary | `#3A3A3C` | Active/pressed state |
| Border subtle | `#38383A` | Dividers, borders |
| Text primary | `#FFFFFF` | Main text |
| Text secondary | `#8E8E93` | Subtitle, metadata |
| Text tertiary | `#636366` | Placeholder, disabled |
| Accent blue | `#0A84FF` | Links, highlights |

**Theme Studio** (Pro feature):
- Users can create custom themes with full color control
- Pre-defined themes available at [ray.so/themes](https://ray.so/themes)
- Extensions automatically adopt the user's theme -- no per-extension styling needed

### 3.6 Open-Source & Community Resources

| Resource | URL |
|----------|-----|
| **Raycast API Docs** | [developers.raycast.com](https://developers.raycast.com) |
| **Raycast Extensions (open-source)** | [github.com/raycast/extensions](https://github.com/raycast/extensions) |
| **Raycast UIKit (Figma)** | [figma.com/community/file/1239440022662828277](https://www.figma.com/community/file/1239440022662828277) |
| **Raycast Theme Explorer** | [ray.so/themes](https://ray.so/themes) |
| **Icons & Images API** | [developers.raycast.com/api-reference/user-interface/icons-and-images](https://developers.raycast.com/api-reference/user-interface/icons-and-images) |
| **Colors API** | [developers.raycast.com/api-reference/user-interface/colors](https://developers.raycast.com/api-reference/user-interface/colors) |
| **cmdk** (underlying command palette) | [cmdk.paco.me](https://cmdk.paco.me/) |

---

## 4. Cross-Cutting Patterns

### 4.1 Shared Design DNA

All three apps share remarkable similarities:

| Pattern | Arc | Linear | Raycast |
|---------|-----|--------|---------|
| **Primary font** | System (SF Pro) | Inter / Inter Display | Inter 4.0 |
| **Dark mode** | Per-Space theming | LCH-generated, exceptional | System-adaptive |
| **Command palette** | Cmd+T (URL bar) | Cmd+K (cmdk) | Global hotkey (cmdk) |
| **Blur/Glass** | Heavy (native) | Subtle (backgrounds) | Heavy (native) |
| **Animation** | Spring physics | Cubic-bezier, fast | Ease-out, minimal |
| **Border radius** | 8-12px | 8-12px | 12px |
| **Accent approach** | Per-Space color | Single indigo | System blue |
| **Information density** | High (vertical tabs) | Very high (lists) | Maximum (text-only) |

### 4.2 The "Premium" Formula

What makes all three feel expensive:

1. **Restraint** -- very few colors, carefully chosen. No unnecessary decoration.
2. **Consistent spacing** -- 4px/8px grid system throughout.
3. **Motion with purpose** -- animations are fast (100-300ms), spring-based, and always communicate state.
4. **Depth through subtlety** -- 1px borders at low opacity, shallow shadows, not heavy drop-shadows.
5. **Typography precision** -- Inter/Inter Display at carefully chosen sizes with proper weights.
6. **Dark mode as default** -- designed dark-first, not dark as afterthought.
7. **Keyboard-first** -- every action reachable without a mouse.
8. **Native feel** -- respecting platform conventions (macOS vibrancy, system fonts where appropriate).
9. **Information density without clutter** -- tight spacing but clear hierarchy through color and weight.
10. **Semantic color systems** -- colors derived from a small number of inputs, never hardcoded per-element.

### 4.3 CSS Variables Blueprint

A combined design token system inspired by all three:

```css
:root {
  /* Spacing (4px base grid) */
  --space-1: 4px;
  --space-2: 8px;
  --space-3: 12px;
  --space-4: 16px;
  --space-5: 20px;
  --space-6: 24px;
  --space-8: 32px;
  --space-10: 40px;
  --space-12: 48px;
  --space-16: 64px;

  /* Typography */
  --font-sans: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
  --font-display: 'Inter Display', var(--font-sans);
  --font-mono: 'JetBrains Mono', 'SF Mono', 'Cascadia Code', monospace;

  --text-xs: 11px;
  --text-sm: 13px;
  --text-base: 14px;
  --text-lg: 16px;
  --text-xl: 20px;
  --text-2xl: 24px;

  /* Border radius */
  --radius-sm: 4px;
  --radius-md: 6px;
  --radius-lg: 8px;
  --radius-xl: 12px;
  --radius-full: 9999px;

  /* Shadows (dark theme) */
  --shadow-sm: 0 1px 2px rgba(0, 0, 0, 0.2);
  --shadow-md: 0 4px 16px rgba(0, 0, 0, 0.3);
  --shadow-lg: 0 16px 48px rgba(0, 0, 0, 0.4);
  --shadow-xl: 0 24px 80px rgba(0, 0, 0, 0.5);
  --shadow-border: 0 0 0 1px rgba(255, 255, 255, 0.06);

  /* Transitions */
  --ease-default: cubic-bezier(0.25, 0.1, 0.25, 1);
  --ease-spring: cubic-bezier(0.2, 0.9, 0.3, 1.0);
  --ease-bounce: cubic-bezier(0.34, 1.56, 0.64, 1);
  --duration-fast: 100ms;
  --duration-default: 200ms;
  --duration-slow: 300ms;

  /* Dark theme colors (Linear-inspired) */
  --bg-base: #0D0E12;
  --bg-surface: #131416;
  --bg-elevated: #1B1C22;
  --bg-hover: #1F2028;
  --bg-selected: #2A2B33;
  --bg-overlay: rgba(0, 0, 0, 0.5);

  --border-default: #26272C;
  --border-subtle: rgba(255, 255, 255, 0.06);
  --border-focus: var(--accent);

  --text-primary: #EEEEEE;
  --text-secondary: #8A8F98;
  --text-muted: #505258;

  --accent: #5E6AD2;
  --accent-hover: #6E7AE2;
  --accent-glow: rgba(94, 106, 210, 0.3);
}
```

### 4.4 Glassmorphism / Blur Technique Reference

```css
/* Light blur (Arc sidebar style) */
.glass-light {
  background: rgba(255, 255, 255, 0.12);
  backdrop-filter: blur(12px) saturate(180%);
  border: 1px solid rgba(255, 255, 255, 0.15);
}

/* Medium blur (Raycast window style) */
.glass-medium {
  background: rgba(28, 28, 30, 0.85);
  backdrop-filter: blur(24px) saturate(150%);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

/* Heavy blur (decorative background element, Linear style) */
.glass-decorative {
  background: conic-gradient(from 180deg, #08AEEA, #2AF598, #FF5ACD, #5E6AD2);
  filter: blur(60px);
  opacity: 0.15;
  pointer-events: none;
}
```

---

## 5. Sources & Resources

### Articles & Blog Posts
- [Arc Browser: Rethinking the Web Through a Designer's Lens](https://medium.com/design-bootcamp/arc-browser-rethinking-the-web-through-a-designers-lens-f3922ef2133e)
- [How we redesigned the Linear UI (part II)](https://linear.app/now/how-we-redesigned-the-linear-ui)
- [A calmer interface for a product in motion (Linear 2026)](https://linear.app/now/behind-the-latest-design-refresh)
- [Linear UI Refresh Changelog (March 2026)](https://linear.app/changelog/2026-03-12-ui-refresh)
- [The rise of Linear style design: origins, trends, and techniques](https://medium.com/design-bootcamp/the-rise-of-linear-style-design-origins-trends-and-techniques-4fd96aab7646)
- [Linear design: The SaaS trend that's boring and bettering UI](https://blog.logrocket.com/ux-design/linear-design/)
- [Accessible linear design across light and dark modes](https://blog.logrocket.com/how-do-you-implement-accessible-linear-design-across-light-and-dark-modes/)
- [A fresh look and feel (Raycast redesign blog)](https://www.raycast.com/blog/a-fresh-look-and-feel)
- [Raycast for designers (UX Collective)](https://uxdesign.cc/raycast-for-designers-649fdad43bf1)
- [Designing a Command Palette](https://destiner.io/blog/post/designing-a-command-palette/)
- [The Browser Company wants you to build your own internet home](https://www.inverse.com/input/design/the-browser-company-arc-design-interview)
- [Karri Saarinen: 10 Rules for Crafting Products That Stand Out](https://www.figma.com/blog/karri-saarinens-10-rules-for-crafting-products-that-stand-out/)
- [Inside Linear: Why craft and focus still win (First Round Review)](https://review.firstround.com/podcast/inside-linear-why-craft-and-focus-still-win-in-product-building/)
- [Using Arc Browser's CSS custom properties for theming](https://ginger.wtf/posts/creating-a-theme-using-arc/)
- [From Concept to Code: Arc Browser search bar animation in SwiftUI](https://medium.com/@bancarel.paul/from-concept-to-code-reproducing-arc-browsers-search-bar-animation-in-swiftui-cd9fdb60e7a5)

### Design Talks & Podcasts
- [Karri Saarinen on Creativity, Tools, and AI](https://en.ai-creators.tech/media/creative/design-search/)
- [Thomas Paul Mann (Raycast CEO) on quality, YC, and AI](https://scalingdevtools.com/podcast/episodes/raycast)
- [Put the Pro in Productivity with Thomas Paul Mann](https://nesslabs.com/raycast-featured-tool)
- [How Arc Grows: Building The New Window Into The Internet](https://www.howtheygrow.co/p/how-arc-grows)

### Figma & Design Kits
- [Arc Browser Interface (Figma)](https://www.figma.com/community/file/1228728710215940920/arc-browser-interface)
- [Arc Browser Mockup + UI Kit (Figma)](https://www.figma.com/community/file/1206735913962953604)
- [Linear Design System (Figma)](https://www.figma.com/community/file/1222872653732371433)
- [Linear UI Free Kit Recreated (Figma)](https://www.figma.com/community/file/1279162640816574368)
- [Raycast UIKit (Figma)](https://www.figma.com/community/file/1239440022662828277)

### Open-Source Code
- [ArcWTF -- Arc look for Firefox (CSS)](https://github.com/KiKaraage/ArcWTF)
- [EdgyArc-fr -- Firefox + Sidebery Arc theme](https://github.com/artsyfriedchicken/EdgyArc-fr)
- [cmdk -- Command palette library (React)](https://cmdk.paco.me/)
- [linear-style -- Community theme index](https://linear.style/)
- [Linear VS Code Theme (color reference)](https://github.com/pabueco/linear-vscode-theme)
- [Raycast Extensions (open-source)](https://github.com/raycast/extensions)

### Official Documentation
- [Raycast Developer API -- User Interface](https://developers.raycast.com/api-reference/user-interface)
- [Raycast Developer API -- Colors](https://developers.raycast.com/api-reference/user-interface/colors)
- [Raycast Developer API -- Icons & Images](https://developers.raycast.com/api-reference/user-interface/icons-and-images)
- [Linear Brand Colors (Mobbin)](https://mobbin.com/colors/brand/linear)
- [Arc Spaces Documentation](https://resources.arc.net/hc/en-us/articles/19228064149143)
- [Arc Pinned Tabs Documentation](https://resources.arc.net/hc/en-us/articles/19231060187159)
- [Arc Favorites Documentation](https://resources.arc.net/hc/en-us/articles/19230755904151)

---

*Document compiled: March 29, 2026*
*For UI implementation reference. Values marked "estimated" are derived from inspection, community recreations, and VS Code theme ports -- not official specifications.*
