# Jarvis / Friday UI Research & Inspiration Document

> Comprehensive reference for designing a desktop application that captures the Marvel Cinematic Universe's holographic interface aesthetic.
> Research conducted 2026-03-29.

---

## Table of Contents

1. [Film Design Analysis](#1-film-design-analysis)
2. [Color System](#2-color-system)
3. [Typography](#3-typography)
4. [Core Visual Patterns](#4-core-visual-patterns)
5. [Animation & Motion Language](#5-animation--motion-language)
6. [FUI Industry Knowledge](#6-fui-industry-knowledge)
7. [Translating FUI to a Real Product](#7-translating-fui-to-a-real-product)
8. [Libraries & Frameworks](#8-libraries--frameworks)
9. [GitHub Repos & Open Source](#9-github-repos--open-source)
10. [CSS Techniques Reference](#10-css-techniques-reference)
11. [3D & WebGL Stack](#11-3d--webgl-stack)
12. [Desktop App (Tauri) Integration](#12-desktop-app-tauri-integration)
13. [Implementation Roadmap](#13-implementation-roadmap)

---

## 1. Film Design Analysis

### 1.1 The People Behind the Interfaces

The MCU's interface work was done by a small number of elite FUI (Fantasy User Interface) studios:

- **Jayse Hansen** -- Designed the actual Iron Man HUD graphics for The Avengers, Iron Man 3, and subsequent films. Created the "HUD Bible," a physical 11x17" printed book documenting every component across all suit iterations (Mk I through Mk VII+). His work appeared in over 70% of The Avengers' screen time. ([Portfolio](https://jayse.tv/v2/?portfolio=hud-2-2)) ([Interview](https://thenextweb.com/news/jayse-hansen-on-creating-tools-the-avengers-use-to-fight-evil-touch-interfaces-and-project-glass))
- **Perception NYC** -- Emmy-nominated design lab that created 125+ shots for Iron Man 2, including Tony's holographic workshop and the "smart-glass" coffee table interface. They pioneered "Science Fiction Thinking" as a design methodology. ([Iron Man 2 work](https://www.experienceperception.com/work/iron-man-2/))
- **Territory Studio** -- Created UI for Avengers: Age of Ultron, Guardians of the Galaxy, and Ex Machina. Their work involved deep research into aviation technology, clinical reconstruction, and cellular biology scanning to make interfaces feel authentic. ([Studio](https://territorystudio.com/sci-fi-interfaces-and-emerging-technology-4/))

### 1.2 The HUD Architecture

The Iron Man HUD follows a specific spatial model documented by [Sci-Fi Interfaces](https://scifiinterfaces.com/2015/07/01/iron-man-hud-a-breakdown/):

**Spherical Layout Model**
- UI elements sit on an invisible sphere surrounding Tony's head
- Stacked elements use concentric invisible spheres at different depths
- Elements slide around the sphere's surface, always facing the viewer
- This ensures all elements are optimally viewable at all times

**Information Hierarchy**
- Small dashboard gauges remain in peripheral vision when not needed
- Elements become larger and more central when contextually relevant
- JARVIS anticipates informational needs and promotes/demotes elements automatically
- The diagnostic widget is a "radial Swiss-army knife" that expands/collapses to whatever tier of information is required

**Core HUD Components**
| Component | Function | Visual Form |
|-----------|----------|-------------|
| Diagnostic Radial | Primary info widget | Expanding/collapsing concentric rings |
| Targeting Reticle | Aim/lock system | Dimensional lens/combat scope, tracks eye |
| Radar/Lidar | Spatial awareness | 2D Synthetic Aperture + 3D detailed view |
| Status Gauges | Suit vitals | Peripheral arc-shaped meters |
| Alert System | Threat detection | Color-shift + center-screen promotion |
| Data Overlays | Contextual info | Layered translucent panels |

**Design Evolution: JARVIS to FRIDAY to EDITH**
- Each AI iteration anticipates graphical feedback needs based on environment context, task purpose, and urgency
- JARVIS (cool blue, analytical) -> FRIDAY (warmer tones, more streamlined) -> EDITH (threat-focused, compact)
- The visual language evolves from complex/layered to more focused/minimal as Tony's suit technology matures

### 1.3 The Holographic Workshop

The workshop interfaces (designed primarily by Perception for Iron Man 2) established key patterns:
- **3D manipulation in mid-air** -- objects rotate, scale, and explode into component views
- **Gesture-driven interaction** -- swipe to dismiss, pinch to zoom, throw to send
- **Transparent layering** -- multiple data planes visible simultaneously at different depths
- **Environmental awareness** -- interfaces integrate with physical objects and surfaces

### 1.4 What Makes It Feel "Jarvis"

The signature aesthetic comes from the combination of:
1. **Blue wireframe motion graphics** on a near-black background
2. **Concentric rings and arcs** that rotate at different speeds
3. **Data that appears to have mass** -- elements drift, settle, and respond to physics
4. **Micro-text that is contextually accurate** -- every tiny label relates to the current story point
5. **Stereo depth** (first used in The Avengers) -- elements exist at different Z-depths
6. **Glow and light bleed** -- elements emit light onto Tony's face
7. **Scan-line artifacts** -- subtle interference patterns suggesting holographic projection

---

## 2. Color System

### 2.1 JARVIS Palette (Cool/Analytical)

The primary JARVIS palette is built around cyan-blue with high contrast against near-black:

```css
:root {
  /* === JARVIS Core === */
  --jarvis-bg-deep:       #000000;   /* True black background */
  --jarvis-bg-surface:    #0a0e17;   /* Dark navy surface */
  --jarvis-bg-elevated:   #0d1b2a;   /* Slightly lifted panels */

  --jarvis-primary:       #00d4ff;   /* Core cyan -- the signature color */
  --jarvis-primary-bright:#00f0ff;   /* Highlighted/active state */
  --jarvis-primary-dim:   #006b80;   /* Muted/inactive elements */
  --jarvis-primary-ghost: #00d4ff1a; /* 10% opacity for backgrounds */

  --jarvis-secondary:     #1ac6ff;   /* Secondary accent -- slightly warmer */
  --jarvis-accent:        #39ff14;   /* Neon green -- alerts/confirmations */
  --jarvis-warning:       #ff6b35;   /* Orange -- warnings */
  --jarvis-danger:        #ff1744;   /* Red -- critical alerts */

  --jarvis-text:          #e0f7ff;   /* Primary text */
  --jarvis-text-dim:      #4a8ea8;   /* Secondary/label text */
  --jarvis-text-micro:    #1e5a73;   /* Micro-text, barely visible */

  --jarvis-border:        #00d4ff33; /* 20% opacity borders */
  --jarvis-glow:          #00d4ff66; /* Glow shadow color */
}
```

### 2.2 FRIDAY Palette (Warmer/Approachable)

FRIDAY's interface shifts toward warmer tones while maintaining the holographic feel:

```css
:root {
  /* === FRIDAY Variant === */
  --friday-primary:       #4fc3f7;   /* Softer, warmer blue */
  --friday-secondary:     #ff9800;   /* Warm orange accent */
  --friday-accent:        #ffb74d;   /* Gold highlights */
  --friday-surface:       #0d1117;   /* Slightly warm black */
}
```

### 2.3 Community-Sourced JARVIS Palette

From [color-hex.com/color-palette/80644](https://www.color-hex.com/color-palette/80644), a five-color JARVIS reference:

| Role | Hex | Usage |
|------|-----|-------|
| Deep Blue | `#0a1f44` | Background depth |
| Soft Cyan | `#00c9ff` | Primary elements |
| Neon Blue | `#1ac6ff` | Active/highlighted |
| Electric Cyan | `#00ffff` | Maximum emphasis |
| White | `#ffffff` | Critical text/alerts |

### 2.4 Glow Implementation

The glow effect is achieved through layered shadows:

```css
.jarvis-element {
  color: var(--jarvis-primary);
  text-shadow:
    0 0 7px  var(--jarvis-primary),
    0 0 10px var(--jarvis-primary),
    0 0 21px var(--jarvis-primary),
    0 0 42px var(--jarvis-glow);
}

.jarvis-panel {
  border: 1px solid var(--jarvis-border);
  box-shadow:
    0 0 5px  var(--jarvis-glow),
    0 0 10px var(--jarvis-glow),
    inset 0 0 5px var(--jarvis-glow);
}
```

---

## 3. Typography

### 3.1 Film-Accurate Fonts

- **Arame Mono** -- The actual font used in the Iron Man Mk VII HUD. Originally designed by Dimitre Lima (DMTR.ORG), with Bold + Mono versions created specifically for Jayse Hansen's Avengers work. Also used in Halo 4. Commercial font via [Fontspring](https://www.fontspring.com/fonts/hitype/0arame-mono).

### 3.2 Recommended Free Alternatives (Google Fonts)

| Font | Style | Best For | Link |
|------|-------|----------|------|
| **Orbitron** | Geometric sans-serif | Headlines, status labels | [Google Fonts](https://fonts.google.com/specimen/Orbitron) |
| **Rajdhani** | Square-shaped sans-serif | UI labels, data values | [Google Fonts](https://fonts.google.com/specimen/Rajdhani) |
| **Share Tech Mono** | Monospace | Code, data readouts, logs | [Google Fonts](https://fonts.google.com/specimen/Share+Tech+Mono) |
| **Exo 2** | Geometric sans-serif | Body text, descriptions | [Google Fonts](https://fonts.google.com/specimen/Exo+2) |
| **Space Mono** | Retro-futuristic mono | Terminal output, counters | [Google Fonts](https://fonts.google.com/specimen/Space+Mono) |
| **Electrolize** | Tech sans-serif | Secondary headings | [Google Fonts](https://fonts.google.com/specimen/Electrolize) |

### 3.3 Typography Rules for FUI

- **All caps for status labels** -- `SYSTEM ONLINE`, `SCANNING...`
- **Monospace for data** -- numbers, percentages, coordinates
- **Letter-spacing: 0.1-0.2em** on labels for that spread-out technical feel
- **Font weight: 300-400** for most elements (thin/light reads as "precise")
- **Small font sizes are fine** -- FUI celebrates density; micro-text at 9-10px adds realism
- **Never use serif fonts** -- they break the futuristic illusion instantly

---

## 4. Core Visual Patterns

### 4.1 Concentric Ring / Arc Reactor Pattern

The most iconic Jarvis element. Rings rotate at different speeds and directions, with data displayed along arcs:

```
        ╭──────────────╮
       ╱   ◠◠◠◠◠◠◠◠◠   ╲
      │  ╭───────────╮  │
      │  │  ●  CORE  │  │    <- Central status
      │  ╰───────────╯  │
       ╲   ◡◡◡◡◡◡◡◡◡   ╱
        ╰──────────────╯
    ← Ring 1 (slow CW) →
  ← Ring 2 (medium CCW) →
← Ring 3 (fast CW, dashed) →
```

Implementation: SVG circles with `stroke-dasharray` and CSS `animation: rotate` at different durations. Use `animateTransform` for multiple concentric circles rotating at different speeds.

### 4.2 Hexagonal Grid

Hexagons tessellating across backgrounds, hovering to cyan glow. Used for status boards, threat maps, and data clustering.

- CSS `clip-path: polygon()` for each hex cell
- GPU-accelerated via WebGL for large grids
- Hover effect: border glow + content reveal
- [CodePen reference](https://codepen.io/bearies/pen/VxxpEr)

### 4.3 Scanning / Processing Animations

The "data scan" effect uses a moving line that reveals information:

```css
.scan-line {
  position: absolute;
  width: 100%;
  height: 2px;
  background: linear-gradient(
    90deg,
    transparent 0%,
    var(--jarvis-primary) 50%,
    transparent 100%
  );
  animation: scan 3s ease-in-out infinite;
}

@keyframes scan {
  0%   { top: 0%; opacity: 0; }
  10%  { opacity: 1; }
  90%  { opacity: 1; }
  100% { top: 100%; opacity: 0; }
}
```

### 4.4 Node / Network Graphs

Tony's interfaces frequently show interconnected data as force-directed networks:
- Nodes with glowing borders
- Animated particles traveling along edges (data flow)
- Nodes scale based on importance
- Click to expand node into full panel
- Physics-based layout with gentle drift

### 4.5 Circular Progress / Status Rings

SVG-based rings using `stroke-dashoffset` for progress:

```css
.progress-ring {
  transform: rotate(-90deg);
  /* Circle r=45, circumference ~283 */
  stroke-dasharray: 283;
  stroke-dashoffset: 70; /* 75% progress */
  transition: stroke-dashoffset 0.5s ease;
}
```

### 4.6 Layered Information Panels

Panels that overlap with transparency, creating depth:
- Background panel at 5% opacity
- Mid-ground at 15% with border
- Foreground content at full opacity
- `backdrop-filter: blur(4px)` on elevated panels

### 4.7 Corner-Bracket Frames

The distinctive FUI panel framing with visible corners but open edges:

```css
.fui-frame {
  position: relative;
  padding: 16px;
}
.fui-frame::before,
.fui-frame::after {
  content: '';
  position: absolute;
  width: 20px;
  height: 20px;
  border-color: var(--jarvis-primary);
  border-style: solid;
}
.fui-frame::before {
  top: 0; left: 0;
  border-width: 1px 0 0 1px; /* top-left corner */
}
.fui-frame::after {
  bottom: 0; right: 0;
  border-width: 0 1px 1px 0; /* bottom-right corner */
}
```

---

## 5. Animation & Motion Language

### 5.1 Principles from the Films

- **Elements animate on different layers in 3D** -- foreground moves faster than background
- **Circular reticles expand and approach from different depths**
- **The HUD swipes to switch modes** (e.g., "battle mode") -- a full-screen transition
- **Nothing is static** -- even idle elements have subtle drift, rotation, or pulse
- **Data reveals itself** -- typewriter text, progressive scan, cascade fill

### 5.2 Timing & Easing

| Action | Duration | Easing |
|--------|----------|--------|
| Panel appear | 300-500ms | `cubic-bezier(0.16, 1, 0.3, 1)` (ease-out expo) |
| Element highlight | 150ms | `ease-out` |
| Ring rotation (idle) | 20-60s | `linear` (continuous) |
| Scan line sweep | 2-4s | `ease-in-out` |
| Text typewriter | 30-50ms per char | `linear` |
| Alert flash | 200ms on / 200ms off | `step-end` |
| Data cascade | 50ms stagger per item | `ease-out` |
| Mode transition | 600-800ms | `cubic-bezier(0.77, 0, 0.175, 1)` |

### 5.3 Staggered Reveals (Framer Motion)

For React implementations, Framer Motion's `staggerChildren` creates the cascade effect:

```tsx
const container = {
  hidden: { opacity: 0 },
  show: {
    opacity: 1,
    transition: {
      staggerChildren: 0.05,
      delayChildren: 0.2,
    },
  },
};

const item = {
  hidden: { opacity: 0, y: 10, filter: "blur(4px)" },
  show: {
    opacity: 1,
    y: 0,
    filter: "blur(0px)",
    transition: { duration: 0.4, ease: "easeOut" },
  },
};
```

### 5.4 Key Animation Effects

- **Breathing pulse**: `opacity` oscillation between 0.6 and 1.0 over 2-3s
- **Ring rotation**: multiple SVG circles with `animateTransform type="rotate"`
- **Particle drift**: small dots with randomized translateX/Y and slow opacity fade
- **Glitch flicker**: rapid opacity changes (0 -> 1 -> 0.5 -> 1) over 100ms
- **Data stream**: vertical scroll of monospace text at constant speed
- **Holographic interference**: horizontal scan lines moving slowly downward

---

## 6. FUI Industry Knowledge

### 6.1 What Is FUI?

"Fantasy User Interface" (or "Fictional User Interface") is the industry term for screen graphics designed for film, TV, and games. Also called:
- **FUI** (most common in the industry)
- **Screen Graphics** (Territory Studio's preferred term)
- **HUDs & GUIs** (the community site [hudsandguis.com](https://www.hudsandguis.com/))
- **UI Props** (on-set terminology)

### 6.2 Key Resources

| Resource | What It Is | URL |
|----------|-----------|-----|
| Sci-Fi Interfaces | Academic analysis of every interface in sci-fi film | [scifiinterfaces.com](https://scifiinterfaces.com/) |
| HUDS+GUIS | Curated gallery of film/TV FUI work | [hudsandguis.com](https://www.hudsandguis.com/) |
| Jayse Hansen Portfolio | The actual Avengers/Iron Man HUD designer | [jayse.tv](https://jayse.tv/v2/?portfolio=hud-2-2) |
| Territory Studio | Avengers: Age of Ultron, Blade Runner 2049 | [territorystudio.com](https://territorystudio.com/project-category/screen-graphics/) |
| Perception NYC | Iron Man 2, Avengers: Endgame | [experienceperception.com](https://www.experienceperception.com/work/iron-man-2/) |

### 6.3 Common FUI Design Patterns

From studying dozens of film interfaces:

1. **Circular/Radial layouts** -- arcs, rings, radar sweeps
2. **Hexagonal grids** -- status boards, threat assessment
3. **Particle systems** -- ambient atmosphere, data flow
4. **Scan lines** -- horizontal interference, holographic artifact
5. **Corner brackets** -- frames that suggest containment without full borders
6. **Micro-typography** -- dense small text that adds realism
7. **Wireframe 3D objects** -- rotating meshes, exploded views
8. **Animated data streams** -- scrolling numbers, waveforms
9. **Color-coded severity** -- blue (nominal) -> yellow (caution) -> red (critical)
10. **Progressive disclosure** -- expand-on-demand radial menus

### 6.4 FUI Design Process (from Jayse Hansen)

1. **Research** -- Fill notebooks studying real military/aviation HUDs
2. **Paper sketches** -- Rough layouts and component ideas
3. **Illustrator** -- Create individual vector elements (most flexible for radial panels, complex icons, grid layouts)
4. **Cinema 4D** -- 3D elements for depth
5. **After Effects** -- Animate, composite, color correct
6. **HUD Bible** -- Document every element's purpose and evolution

---

## 7. Translating FUI to a Real Product

### 7.1 The Fundamental Tension

Film FUI is designed to be **looked at for 2-5 seconds** at a time. A real product must be **used for hours**.
The challenge: capture the "wow factor" while maintaining the readability and usability of a professional tool.

### 7.2 Design Principles for Functional FUI

From [Sarah Kay Miller's analysis](https://medium.com/domo-ux/designing-a-functional-futuristic-user-interface-c27d617ce8cc):

1. **Make users feel powerful** -- FUI conveys intelligence and mastery. Your product should authentically deliver that, not just simulate it.
2. **Balance boredom vs. anxiety** -- Too simple = disengaging. Too complex = overwhelming. Target the "flow" state.
3. **Consistency with strategic novelty** -- Keep chart types consistent for quick comprehension, but introduce visual variety to prevent monotony.
4. **Function precedes aesthetics** -- Every visual element must serve a purpose. Decorative-only elements quickly become noise.
5. **Respect cognitive load** -- Dense data is fine if it's organized with clear hierarchy.

### 7.3 Practical Usability Guidelines

| Film FUI Pattern | Real Product Adaptation |
|------------------|------------------------|
| Blue-on-black everything | Add a subtle dark gray surface (`#0d1b2a`) to distinguish panels |
| Text at 8px | Minimum 12px for body, 10px only for decorative micro-text |
| Constant animation | Animate on state change only; idle = minimal motion |
| No contrast hierarchy | Use opacity levels (100%, 70%, 40%) for clear information ranking |
| Full-screen takeover | Panels and modals with clear dismiss affordance |
| Everything glows | Reserve glow for active/important elements only |

### 7.4 Accessibility Checklist

Even with a dark sci-fi theme, meet these minimums:

- **Contrast ratio**: 4.5:1 minimum for body text (WCAG AA). Cyan `#00d4ff` on `#0a0e17` = ~10:1. Passes easily.
- **Focus indicators**: Visible glow ring on keyboard focus (natural fit for this aesthetic)
- **Reduced motion**: Respect `prefers-reduced-motion` -- disable ring rotation, particle effects, scan lines
- **Screen readers**: All interactive elements need ARIA labels; decorative SVGs get `aria-hidden="true"`
- **Font sizing**: Use `rem` units; support browser zoom to 200%

### 7.5 Performance Budget

For 60fps on mid-range hardware:

| Effect | Cost | Budget |
|--------|------|--------|
| CSS animations (opacity, transform) | Cheap (GPU-composited) | Unlimited |
| box-shadow glow | Medium (paint) | < 20 active at once |
| backdrop-filter blur | Medium-Heavy | < 5 active panels |
| SVG animation (rings) | Light | < 10 animated rings |
| Three.js particle system | Heavy | < 10K particles |
| Three.js force graph | Heavy | < 500 nodes with bloom |
| WebGL post-processing (bloom) | Heavy | 1 bloom pass max |

---

## 8. Libraries & Frameworks

### 8.1 Arwes -- Sci-Fi UI Web Framework

The most complete FUI framework available.

| Property | Detail |
|----------|--------|
| Stars | 7.5K on GitHub |
| Stack | TypeScript 94%, React 18+ |
| Status | **Alpha** -- not production-ready, API may change |
| Inspiration | Cyberprep, Star Citizen, Halo, TRON: Legacy, NIKKE |
| License | MIT |

**Components**: FrameBox, FrameCorners, FramePentagon, FrameHexagon, FrameLines, Text, Animator (transition system), Bleeps (sound effects)

**Key Features**:
- Built-in animation system with `animate` and `show` props
- Frame components build containers from configurable polylines/polygons
- Sound effects integration for immersive experience
- Works with Next.js and Remix (but NOT React strict mode or React Server Components)

**Caveats**: Alpha quality. 15+ community projects use it, but expect API churn.

- [Docs](https://arwes.dev/docs)
- [GitHub](https://github.com/arwes/arwes)
- [Animation System](https://version1-breakpoint1.arwes.dev/docs/animation-system)

### 8.2 Dynamic SciFi Dashboard Kit

Lightweight, dependency-free library with ready-made sci-fi panel components.

| Property | Detail |
|----------|--------|
| Stack | Vanilla ES6+ JavaScript, CSS variables |
| Dependencies | None |
| License | Open source |

**13 Panel Components**:
1. `LogDisplayPanel` -- scrolling color-coded log
2. `CriticalWarningTextPanel` -- animated warnings
3. `KeyValueListPanel` -- styled data tables
4. `LedDisplayPanel` -- digital LED text
5. `DynamicTextPanel` -- typewriter effects
6. `ActionButtonsPanel` -- styled button groups
7. `CanvasGraphPanel` -- basic line/bar charts
8. `IntegrityPulsePanel` -- animated status indicators
9. `CircularGaugePanel` -- radial progress
10. `StatusIndicatorLedPanel` -- multi-state LEDs
11. `HorizontalBarGaugePanel` -- progress bars
12. `TrueCanvasGraphPanel` -- advanced charting
13. `ImageDisplayPanel` -- sci-fi framed images

Built-in scanlines, sparks, and canvas-based hardware-accelerated rendering.

- [Demo](https://www.cssscript.com/dynamic-scifi-dashboard/)
- [GitHub](https://github.com/soyunomas/Dynamic-SciFi-Dashboard-Kit)

### 8.3 Cosmic UI

Modern TailwindCSS-based approach with SVG-first architecture.

| Property | Detail |
|----------|--------|
| Stack | TailwindCSS, SVG, zag.js for interactions |
| Framework | Agnostic (React, Vue, Solid) |
| Focus | Accessible sci-fi components |

**Key Features**:
- SVG-first architecture with customizable shapes
- zag.js integration for accessible state management (keyboard + ARIA)
- Holographic effects: built-in glows, gradients
- Angled corners with beveled edges (spaceship control panel aesthetic)
- Animated gradients with particle-like animations

- [Docs](https://www.cosmic-ui.com/docs)
- [GitHub](https://github.com/rizkimuhammada/cosmic-ui)

### 8.4 Holo (Vue 3)

Vue 3 component library for holographic UIs.

- Attempts to balance usability, customization flexibility, and eye candy
- [GitHub](https://github.com/OviOvocny/Holo)

### 8.5 CSS-sci-fi-ui

CSS-only framework for sci-fi styling with no JavaScript dependency.

- [GitHub](https://github.com/MYRWYR/CSS-sci-fi-ui)

### 8.6 Futurism (Tailwind Dashboard)

Commercial Tailwind CSS dashboard template with sleek dark mode and subtle animations.

- [Demo](https://futurism.tailwinddashboard.com/)
- [Info](https://tailwinddashboard.com/futurism-template/)

---

## 9. GitHub Repos & Open Source

### 9.1 Full JARVIS Implementations

| Repo | Stack | Features |
|------|-------|----------|
| [harsh-raj00/my-jarvis](https://github.com/harsh-raj00/my-jarvis) | React + Three.js + FastAPI + Gemini AI | Arc Reactor 3D, holographic particle sphere (8K particles), rotating rings, voice visualization, WebSocket real-time, ElevenLabs TTS, 6 plugins, GLSL shaders |
| [cam-hm/jarvis](https://github.com/cam-hm/jarvis) | FastAPI + Gemini + Three.js | Voice-activated, holographic Arc Reactor, custom JARVIS voice model |
| [ishaan1013/jarvis](https://github.com/ishaan1013/jarvis) | Web | Interactive gesture-controlled 3D hologram |

### 9.2 3D Graph Visualization

| Repo | Stack | Why It Matters |
|------|-------|----------------|
| [vasturiano/3d-force-graph](https://github.com/vasturiano/3d-force-graph) | Three.js + d3-force-3d | 3D force-directed graphs with directional particles on links, bloom support, custom node/link rendering, orbit/fly controls |
| [vasturiano/react-force-graph](https://github.com/vasturiano/react-force-graph) | React wrapper | 2D/3D/VR/AR modes, built-in post-processing composer for bloom, selective glow via luminanceThreshold |
| [vasturiano/d3-force-3d](https://github.com/vasturiano/d3-force-3d) | d3-force extended | 3D physics engine underlying the graph components |

### 9.3 Holographic & Particle Effects

| Repo | What | Link |
|------|------|------|
| threejs-holographic-material | Holographic material with scanlines, vibrant colors, futuristic brilliance | [GitHub](https://github.com/ektogamat/threejs-vanilla-holographic-material) |
| three-nebula | Particle system engine for Three.js | [GitHub](https://github.com/creativelifeform/three-nebula) |
| three.quarks | General purpose VFX/particle engine for Three.js | [GitHub](https://github.com/Alchemist0823/three.quarks) |
| WebGL-GPU-Particles | 1M+ GPU-accelerated particles | [GitHub](https://github.com/soulwire/WebGL-GPU-Particles) |

### 9.4 Globe & Spatial Visualization

| Repo | What | Link |
|------|------|------|
| globe.gl | Globe data visualization (Three.js) | [GitHub](https://github.com/vasturiano/globe.gl) |

---

## 10. CSS Techniques Reference

### 10.1 Neon Glow (Text)

```css
.neon-text {
  color: #00d4ff;
  text-shadow:
    0 0 7px #00d4ff,
    0 0 10px #00d4ff,
    0 0 21px #00d4ff,
    0 0 42px #0077ff,
    0 0 82px #0077ff;
  font-family: 'Share Tech Mono', monospace;
  letter-spacing: 0.15em;
  text-transform: uppercase;
}
```

### 10.2 Holographic Scan Lines Overlay

```css
.scanlines::after {
  content: '';
  position: absolute;
  inset: 0;
  background: repeating-linear-gradient(
    0deg,
    transparent,
    transparent 1px,
    rgba(0, 212, 255, 0.03) 1px,
    rgba(0, 212, 255, 0.03) 2px
  );
  pointer-events: none;
  animation: scanlines-drift 8s linear infinite;
}

@keyframes scanlines-drift {
  0%   { background-position: 0 0; }
  100% { background-position: 0 100px; }
}
```

### 10.3 Animated Border Glow

```css
.glow-border {
  border: 1px solid rgba(0, 212, 255, 0.3);
  box-shadow:
    0 0 5px rgba(0, 212, 255, 0.2),
    0 0 10px rgba(0, 212, 255, 0.1),
    inset 0 0 5px rgba(0, 212, 255, 0.05);
  animation: border-pulse 3s ease-in-out infinite;
}

@keyframes border-pulse {
  0%, 100% { box-shadow: 0 0 5px rgba(0, 212, 255, 0.2), 0 0 10px rgba(0, 212, 255, 0.1); }
  50%      { box-shadow: 0 0 10px rgba(0, 212, 255, 0.4), 0 0 20px rgba(0, 212, 255, 0.2); }
}
```

### 10.4 CRT / Holographic Flicker

```css
.holo-flicker {
  animation: flicker 4s infinite;
}

@keyframes flicker {
  0%, 97%, 100% { opacity: 1; }
  97.5%         { opacity: 0.8; }
  98%           { opacity: 1; }
  98.5%         { opacity: 0.6; }
  99%           { opacity: 1; }
}
```

### 10.5 Data Reveal / Typewriter

```css
.typewriter {
  overflow: hidden;
  white-space: nowrap;
  border-right: 2px solid var(--jarvis-primary);
  animation:
    typing 2s steps(40, end),
    blink-caret 0.75s step-end infinite;
}

@keyframes typing {
  from { width: 0; }
  to   { width: 100%; }
}

@keyframes blink-caret {
  from, to { border-color: transparent; }
  50%      { border-color: var(--jarvis-primary); }
}
```

### 10.6 Rotating Ring (SVG + CSS)

```html
<svg viewBox="0 0 200 200" width="200" height="200">
  <circle cx="100" cy="100" r="90" fill="none"
    stroke="#00d4ff" stroke-width="0.5" stroke-dasharray="10 5"
    class="ring ring-slow" />
  <circle cx="100" cy="100" r="70" fill="none"
    stroke="#00d4ff" stroke-width="0.3" stroke-dasharray="3 8"
    class="ring ring-fast" />
</svg>

<style>
.ring {
  transform-origin: center;
}
.ring-slow {
  animation: spin 30s linear infinite;
}
.ring-fast {
  animation: spin 12s linear infinite reverse;
}
@keyframes spin {
  to { transform: rotate(360deg); }
}
</style>
```

### 10.7 CodePen References

| CodePen | What | URL |
|---------|------|-----|
| React Sci-Fi HUD Card | Responsive card component | [codepen.io/acarlie/pen/NWBzjJP](https://codepen.io/acarlie/pen/NWBzjJP) |
| Holographic Effect CSS | Floating holographic transform | [codepen.io/johnlouie04/pen/NeJBwO](https://codepen.io/johnlouie04/pen/NeJBwO) |
| Futuristic HUD Element | Rotating circular elements | [codepen.io/nishit-sarvaiya/pen/qBbomVj](https://codepen.io/nishit-sarvaiya/pen/qBbomVj) |
| SCI-FI UI | Full sci-fi interface layout | [codepen.io/inVoltag/pen/ZPwdoP](https://codepen.io/inVoltag/pen/ZPwdoP) |
| HUD Control Monitor | Orbitron font, radial gradients, 3D transforms | [codepen.io/anthonygermishuys/pen/GJWrPR](https://codepen.io/anthonygermishuys/pen/GJWrPR) |
| Futuristic HUD Interface | Animated lines, glow box-shadows | [codepen.io/jayramoliya/pen/zxYWovb](https://codepen.io/jayramoliya/pen/zxYWovb) |
| Scifi Stuff | Mixed sci-fi elements | [codepen.io/marioluevanos/pen/XKqNZB](https://codepen.io/marioluevanos/pen/XKqNZB) |
| Sci Fi Loader | Rotating discs, glow effects | [codepen.io/hugo/pen/bGVaOGE](https://codepen.io/hugo/pen/bGVaOGE) |
| CSS Scanlines | Pure CSS scan line overlay | [codepen.io/meduzen/pen/zxbwRV](https://codepen.io/meduzen/pen/zxbwRV) |
| CSS CRT Screen | Full CRT effect with flicker | [codepen.io/lbebber/pen/XJRdrV](https://codepen.io/lbebber/pen/XJRdrV) |
| SCIFI UI KIT | Component kit | [codepen.io/Heavybrush/pen/wryYYr](https://codepen.io/Heavybrush/pen/wryYYr) |
| SVG Sci-fi Circle | Concentric rotating rings | [codepen.io/marcuswallberg/pen/ggbZQJ](https://codepen.io/marcuswallberg/pen/ggbZQJ) |
| Animated Hex Pattern | SVG hexagon grid animation | [codepen.io/bearies/pen/VxxpEr](https://codepen.io/bearies/pen/VxxpEr) |

Browsing the [sci-fi tag on CodePen](https://codepen.io/tag/sci-fi) yields hundreds more examples.

---

## 11. 3D & WebGL Stack

### 11.1 Recommended Stack for Node Graph + Holographic Effects

```
React (UI framework)
  ├── @react-three/fiber (R3F) -- React renderer for Three.js
  │     ├── @react-three/postprocessing -- Bloom, selective glow
  │     └── @react-three/drei -- Utilities (Text, Float, Stars)
  ├── react-force-graph-3d -- 3D node graph with physics
  │     └── d3-force-3d -- Underlying physics engine
  └── framer-motion -- 2D UI animations
```

### 11.2 Three.js Post-Processing for Bloom

The key to the Jarvis glow aesthetic is the **UnrealBloomPass**:

```tsx
import { EffectComposer, Bloom } from '@react-three/postprocessing';

<EffectComposer>
  <Bloom
    luminanceThreshold={1}   /* Only materials > 1 brightness glow */
    luminanceSmoothing={0.9}
    intensity={1.5}
    radius={0.8}
  />
</EffectComposer>
```

**Selective Bloom**: Set `luminanceThreshold={1}` so nothing glows by default. Then lift specific materials' emissive colors above 1.0 to make them bloom. This gives you precise control over which elements glow.

### 11.3 3D Force Graph Configuration for Sci-Fi

```tsx
<ForceGraph3D
  backgroundColor="#000008"
  nodeColor={() => '#00d4ff'}
  nodeOpacity={0.9}
  linkColor={() => '#00d4ff33'}
  linkWidth={0.5}
  linkDirectionalParticles={4}
  linkDirectionalParticleSpeed={0.005}
  linkDirectionalParticleWidth={1.5}
  linkDirectionalParticleColor={() => '#00d4ff'}
  enableNodeDrag={true}
  nodeThreeObject={/* custom glowing sphere */}
/>
```

Key features for the Jarvis look:
- `linkDirectionalParticles` -- animated dots traveling along edges (data flow)
- `emitParticle()` -- on-demand particle emission for events
- Custom `nodeThreeObject` -- replace default spheres with glowing ring+sphere combos
- Post-processing composer access for adding bloom

### 11.4 Performance in 2026

Three.js now supports **WebGPU** (production-ready since r171, September 2025):
- ~95% browser coverage (including Safari 26)
- WebGL 2 fallback for the remaining 5%
- **Compute shaders** unlock 10-100x performance for particle systems
- **Instancing and batching** for draw call reduction
- Target: **under 100 draw calls** for smooth 60fps
- Use Three Nebula or three.quarks for managed particle systems

### 11.5 Holographic Material

The [threejs-holographic-material](https://github.com/ektogamat/threejs-vanilla-holographic-material) provides:
- Vibrant holographic colors
- Dynamic scanlines
- Futuristic brilliance effects
- Available for both vanilla Three.js and React Three Fiber

---

## 12. Desktop App (Tauri) Integration

### 12.1 Frameless Transparent Window

For a Jarvis-style desktop app, use Tauri's window customization:

```json
// tauri.conf.json
{
  "windows": [
    {
      "decorations": false,
      "transparent": true,
      "width": 1200,
      "height": 800
    }
  ]
}
```

- Use `data-tauri-drag-region` on custom titlebar HTML elements for window dragging
- No native titlebar -- build custom FUI-styled window controls

### 12.2 Acrylic / Glass Effect

For Windows, use the `window-vibrancy` crate:
- Add to `Cargo.toml` as a dependency
- Call `apply_acrylic()` in setup for Windows 10/11 blur-behind effect
- Combine with CSS translucent backgrounds for glass-morphism panels

**Caveat**: CSS `backdrop-filter: blur()` does NOT see through the Tauri window to the desktop. The acrylic effect must be applied at the native (Rust) level using `window-vibrancy`.

### 12.3 Architecture for Jarvis Desktop

```
Tauri (Rust backend, system tray, native APIs)
  └── WebView (HTML/CSS/JS frontend)
        ├── React + TypeScript
        ├── @react-three/fiber (3D canvas for node graph, particles)
        ├── react-force-graph-3d (AI brain visualization)
        ├── Framer Motion (2D panel animations)
        ├── Custom CSS (glow, scanlines, FUI frames)
        └── WebSocket to backend services
```

---

## 13. Implementation Roadmap

### Phase 1: Foundation (CSS + Design Tokens)

1. Define the complete CSS custom property system (Section 2)
2. Set up typography with Google Fonts (Section 3)
3. Build the FUI frame component (corner brackets, glow borders)
4. Create the scan-line overlay as a global effect
5. Build the panel component with depth layering

### Phase 2: Core Components

1. Status ring (SVG circular progress)
2. Log display (scrolling, color-coded)
3. Data table (KeyValue with sci-fi styling)
4. Alert/warning system with color-coded severity
5. Typewriter text component
6. Button with glow hover state

### Phase 3: Animation Layer

1. Staggered reveal system (Framer Motion)
2. Idle animations (ring rotation, breathing pulse)
3. Mode transitions (full-screen swipe)
4. Reduced-motion variant for accessibility

### Phase 4: 3D Visualization

1. Three.js canvas with R3F
2. Force-directed graph with react-force-graph-3d
3. UnrealBloomPass for selective glow
4. Particle effects (ambient + data flow on edges)
5. Custom node rendering (glowing sphere + ring)

### Phase 5: Desktop Integration

1. Tauri frameless window
2. Custom titlebar with FUI controls
3. Acrylic/vibrancy for glass effect
4. System tray integration
5. WebSocket bridge to backend

---

## Sources

### Primary References
- [Jayse Hansen FUI Portfolio](https://jayse.tv/v2/?portfolio=hud-2-2)
- [Jayse Hansen Interview (The Next Web)](https://thenextweb.com/news/jayse-hansen-on-creating-tools-the-avengers-use-to-fight-evil-touch-interfaces-and-project-glass)
- [Iron Man HUD Breakdown (Sci-Fi Interfaces)](https://scifiinterfaces.com/2015/07/01/iron-man-hud-a-breakdown/)
- [Design of The Avengers (HUDS+GUIS)](https://www.hudsandguis.com/home/2013/05/15/the-avengers)
- [Perception NYC - Iron Man 2](https://www.experienceperception.com/work/iron-man-2/)
- [Territory Studio - Sci-Fi Interfaces](https://territorystudio.com/sci-fi-interfaces-and-emerging-technology-4/)

### Design Theory
- [Designing a Functional Futuristic UI (Sarah Kay Miller)](https://medium.com/domo-ux/designing-a-functional-futuristic-user-interface-c27d617ce8cc)
- [Behind Wireframe: Fantasy UI in Films (Medium)](https://medium.com/thinking-design/behind-wireframe-how-fantasy-ui-in-films-and-tv-mirrors-our-own-design-reality-a20deee1646f)
- [Sci-Fi Interfaces (Book/Site)](https://scifiinterfaces.com/)
- [Beyond the Jarvis Fantasy (Daniel Bentes)](https://medium.com/@danielbentes/beyond-the-jarvis-fantasy-why-sci-fi-got-ai-interfaces-wrong-c6d1d99415d4)

### Libraries
- [Arwes Framework](https://arwes.dev/) / [GitHub](https://github.com/arwes/arwes)
- [Dynamic SciFi Dashboard Kit](https://github.com/soyunomas/Dynamic-SciFi-Dashboard-Kit)
- [Cosmic UI](https://www.cosmic-ui.com/docs) / [GitHub](https://github.com/rizkimuhammada/cosmic-ui)
- [Holo (Vue 3)](https://github.com/OviOvocny/Holo)
- [CSS-sci-fi-ui](https://github.com/MYRWYR/CSS-sci-fi-ui)

### 3D & Graph Visualization
- [3D Force Graph](https://github.com/vasturiano/3d-force-graph)
- [React Force Graph](https://github.com/vasturiano/react-force-graph)
- [Three.js Holographic Material](https://github.com/ektogamat/threejs-vanilla-holographic-material)
- [Three Nebula (Particles)](https://github.com/creativelifeform/three-nebula)
- [three.quarks (VFX)](https://github.com/Alchemist0823/three.quarks)
- [React Postprocessing (Bloom)](https://react-postprocessing.docs.pmnd.rs/effects/bloom)

### JARVIS Implementations
- [harsh-raj00/my-jarvis](https://github.com/harsh-raj00/my-jarvis)
- [cam-hm/jarvis](https://github.com/cam-hm/jarvis)
- [ishaan1013/jarvis](https://github.com/ishaan1013/jarvis)

### CSS Techniques
- [CSS Glow Effects (FreeFrontEnd)](https://freefrontend.com/css-glow-effects/)
- [Neon Text with CSS (CSS-Tricks)](https://css-tricks.com/how-to-create-neon-text-with-css/)
- [CRT Display CSS](https://aleclownes.com/2017/02/01/crt-display.html)
- [Progress Ring (CSS-Tricks)](https://css-tricks.com/building-progress-ring-quickly/)

### Desktop / Tauri
- [Tauri Window Customization](https://v2.tauri.app/learn/window-customization/)
- [Acrylic Window Effect with Tauri](https://dev.to/waradu/acrylic-window-effect-with-tauri-1078)
- [window-vibrancy crate](https://github.com/tauri-apps/window-vibrancy)

### Performance
- [100 Three.js Tips (2026)](https://www.utsubo.com/blog/threejs-best-practices-100-tips)
- [Three.js WebGPU Migration Guide](https://www.utsubo.com/blog/webgpu-threejs-migration-guide)
- [Building Efficient Three.js Scenes (Codrops)](https://tympanus.net/codrops/2025/02/11/building-efficient-three-js-scenes-optimize-performance-while-maintaining-quality/)

### Fonts
- [Arame Mono (Fontspring)](https://www.fontspring.com/fonts/hitype/0arame-mono)
- [Orbitron (Google Fonts)](https://fonts.google.com/specimen/Orbitron)
- [Rajdhani (Google Fonts)](https://fonts.google.com/specimen/Rajdhani)
- [Best Sci-Fi Fonts (Super Dev Resources)](https://superdevresources.com/techno-sci-fi-fonts/)
- [JARVIS Color Palette](https://www.color-hex.com/color-palette/80644)

### Color Palettes
- [JARVIS Color Palette (color-hex.com)](https://www.color-hex.com/color-palette/80644)
- [Iron Man Colors (ColorsWall)](https://colorswall.com/palette/3128)
