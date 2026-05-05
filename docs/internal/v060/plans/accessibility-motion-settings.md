# Cortex v0.6.0 — Accessibility, Motion & Settings Master Plan

**Status:** Research synthesis complete 2026-04-23. Ready for execution.
**Scope lock reference:** `scope-lock.md` Tier 1 headline — blocks v0.6.0 cut.
**Target date:** 2026-07-16 (full v0.6.0 cut). A11y work scheduled weeks 2-9.
**Research inputs:**
- `research/a11y-codebase-audit.md` — current Cortex state (~20% WCAG 2.2 AA today)
- `research/a11y-wcag22-research.md` — WCAG 2.2 AA new criteria + testing matrix
- `research/a11y-motion-research.md` — motion token systems + library comparison
- `research/a11y-react-libs.md` — Radix/React Aria/Headless UI deep dive

---

## 1. Executive summary

### Current state (2026-04-23)

Cortex Control Center today:
- ✓ Semantic landmarks (`<nav>`, `<main>`, `<aside>`)
- ✓ Heading hierarchy mostly sound
- ✓ Some `aria-label` on primary buttons
- ✓ One reduced-motion rule covering ~6 classes
- ✗ 8+ `div`-with-`onClick` missing `tabIndex` + keyboard handlers
- ✗ 5 CSS `outline: none` rules strip focus rings
- ✗ 3 dialogs without `aria-modal`, focus trap, or focus restoration
- ✗ 9 form inputs placeholder-only, no `<label>`
- ✗ 24+ animations unresponsive to `prefers-reduced-motion`
- ✗ `setFeedbackMessage()` called 100+ times without `aria-live`
- ✗ 0 a11y tests across 27 unit tests (6 suites)
- ✗ `accessibility-checker-engine` installed but unused

Result: **~20% WCAG 2.2 AA coverage**. Not accessible to keyboard-only or screen-reader users today.

### Target at v0.6.0 cut

- WCAG 2.2 AA compliance across all main flows (9 new success criteria addressed — see §3)
- 100% keyboard operability
- Screen reader verified on NVDA+Firefox (primary), VoiceOver+Safari (secondary), Narrator+Edge (smoke)
- `prefers-reduced-motion` honored at runtime (plus 3-state Settings override)
- `axe-core` automated gate in CI — zero critical/serious violations on main flows
- First-class **Settings panel** with 4 sections: Accessibility, Appearance & Motion, Connection, Keyboard & Navigation
- Central motion token module powering ~8kB-gzip animation stack
- Dialog semantics fixed via Radix primitives

### Stack picks (locked)

| Layer | Choice | Why |
|-------|--------|-----|
| Primitives | **Radix UI** (`radix-ui` unified package) | MIT, JSX-native, WorkOS-maintained, shadcn ecosystem fit |
| Dialog | **Radix Dialog** | Focus trap, portal, escape, scroll lock — all built in |
| Tabs | **Radix Tabs** (`activationMode="manual"`) | Roving tabindex + manual activation matches settings sidebar |
| Combobox | **Downshift `useCombobox`** (if needed) | JSX-friendly, ARIA 1.2, ~3.5kB gzip |
| Focus trap (non-dialog) | **react-focus-lock** | 5kB gzip, React 19 ready, portal-aware (rejected focus-trap-react as strictly worse) |
| Toasts | **Sonner** | Correct `aria-live` defaults, 4kB gzip |
| Live regions | **Custom `useAnnouncer` hook** (~30 LOC) | react-aria-live unmaintained |
| Forms | **React Hook Form + custom `FormErrorSummary`** | Proper error summary + live-region announcement |
| Motion tokens | **CSS custom properties** | 80% of animation need; no JS dependency |
| Motion (lists) | **`@formkit/auto-animate`** | ~3kB, auto reduced-motion, zero config |
| Motion (modals/routes) | **`motion/react`** via `m`+`LazyMotion` | ~4.6kB gzip (up to 34kB full bundle) |
| Reduced-motion hook | **Custom `useReducedMotion`** via `useSyncExternalStore` | Hydration-safe, React 18+ |
| Lint | **`eslint-plugin-jsx-a11y/strict`** (flat config, v6.10.2) | WCAG 2.2 AA rule mapping |
| Unit tests | **`vitest-axe`** + `@testing-library/jest-dom` matchers | Vite + React fit |
| Dev-mode runtime | **`@axe-core/react`** | Runtime audit during development |
| E2E tests | **`@axe-core/playwright`** | Full WCAG 2.2 coverage, matrix across WebView2/WKWebView/WebKitGTK |

**Rejected (with reasons):**
- ❌ **Reach UI** — unmaintained since Sept 2022
- ❌ **react-modal** — stale (Oct 2022), React 19 risk
- ❌ **React Aria Components** — tree-shaking unreliable (175KB for one Button in Next.js reports), JSX ergonomics degrade without TS
- ❌ **Headless UI** — slower cadence, Tailwind-coupled mindset, smaller palette
- ❌ **Base UI v1** — legitimate but only 5 months since v1.0; re-evaluate for v0.7.0
- ❌ **focus-trap-react** — strictly worse than react-focus-lock for Radix-free scenarios
- ❌ **APCA contrast** — still WCAG 3 Working Draft; don't claim "WCAG 3 compliant"
- ❌ **JAWS testing** — deferred to v0.7.0 (market share ~35% but NVDA free and covers same patterns)

### Total bundle impact

| Library | Gzipped | Note |
|---------|---------|------|
| Radix UI (Dialog + Tabs + Toast) | ~25kB | Per-primitive tree-shaken |
| react-focus-lock | ~5kB | Non-dialog focus traps |
| Sonner | ~4kB | Toast primitive |
| Downshift (if adopted) | ~3.5kB | Combobox only |
| `@formkit/auto-animate` | ~3kB | Lists |
| `motion/react` (`m` + LazyMotion) | ~4.6kB | Modals + routes |
| **Total** | **~45kB gzipped** | Negligible for Tauri desktop |

### Timeline

**Weeks 2-9 of v0.6.0 cycle (8 weeks total a11y work):**

- **Week 2 (Sprint A, Critical):** Focus, keyboard, dialogs, labels, live region, reduced-motion — ~40 hrs
- **Weeks 3-4 (Sprint B, Settings):** Settings panel skeleton + 4 sections + persistence — ~40 hrs
- **Weeks 5-6 (Sprint C, Motion):** Motion token system + library adoption + sidebar/tab/panel transitions — ~40 hrs
- **Weeks 7-8 (Sprint D, Testing):** axe-core CI gate + vitest-axe unit + Playwright E2E + manual walkthroughs — ~40 hrs
- **Week 9 (Sprint E, Polish):** Contrast pass, ARIA completeness, remaining motion coverage, docs — ~20 hrs

**Total effort:** ~180 hours / ~4.5 engineer-weeks of focused work. Fits Tier 1 headline budget.

---

## 2. WCAG 2.2 AA — new success criteria mapping

Nine new criteria in WCAG 2.2 (published 2023-10). Cortex applicability:

| SC | Name | Level | Applies to Cortex? | Plan |
|----|------|-------|---------------------|------|
| 2.4.11 | Focus Not Obscured (Minimum) | AA | Yes — sticky headers + Settings sidebar | Sprint A: verify focus ring visible past sticky elements |
| 2.4.12 | Focus Not Obscured (Enhanced) | AAA | Not required (AAA) | Skip |
| 2.4.13 | Focus Appearance | AAA | Not required (AAA) | Skip; Sprint A meets Minimum via tokens |
| 2.5.7 | Dragging Movements | AA | Partial — BrainVisualizer node drag | Sprint C: add keyboard alternative (arrow keys move selected node) |
| 2.5.8 | Target Size (Minimum) | AA | Yes — all interactive elements | Sprint A: audit, enforce **32×32 minimum** (above AA 24, matches macOS HIG) |
| 3.2.6 | Consistent Help | A | Yes — Settings has "Help" links | Sprint B: ensure Help appears same location every screen |
| 3.3.7 | Redundant Entry | A | Partial — Connection dialog could prefill | Sprint B: prefill host/port from last successful connection |
| 3.3.8 | Accessible Authentication (Minimum) | AA | Not yet — local-first, no auth UI | Deferred — WebAuthn/passkey if sync ships later |
| 3.3.9 | Accessible Authentication (Enhanced) | AAA | Not required | Skip |

**Note:** 4.1.1 Parsing (from WCAG 2.0) was **removed** in 2.2 — obsolete. No action.

---

## 3. Sprint A — Critical remediation (Week 2, ~40 hours)

Unblocks every downstream sprint. All P0 critical gaps closed here.

### A.1 — Focus system rebuild (~4 hrs)

**Problem:** 5 CSS `outline: none` rules strip focus (styles.css:636, 709, 892, 3432, 3909). Makes keyboard use invisible.

**Solution:** CSS custom properties + `:focus-visible` pattern:

```css
:root {
  --focus-ring-width: 2px;
  --focus-ring-color: var(--cyan-400);
  --focus-ring-offset: 2px;
  --focus-ring-shadow: 0 0 0 4px rgba(0, 212, 255, 0.2);
}

*:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring-color);
  outline-offset: var(--focus-ring-offset);
  box-shadow: var(--focus-ring-shadow);
}

/* Remove all outline: none rules; replace with :focus-visible where needed */
```

**Files:**
- `desktop/cortex-control-center/src/styles.css` — delete lines 636, 709, 892, 3432, 3909; add `:root` tokens + global `:focus-visible` rule near top of file
- `desktop/cortex-control-center/src/design/tokens/focus.css` *(new)* — extract tokens

**Verification:**
- Manually Tab through every interactive element; focus ring visible
- Screenshot diff: before/after on primary views

### A.2 — Keyboard navigation (~8 hrs)

**Problem:** 8+ `div`-with-`onClick` not reachable or activatable by keyboard.

**Locations (from audit):**
- `App.jsx:3945` — sidebar nav items
- `App.jsx:4294` — memory card
- `App.jsx:4061` — Connection overlay
- `App.jsx:4125` — Connection dialog backdrop
- `App.jsx:4349` — metric legend toggle
- `BrainVisualizer.jsx:314` — Brain 2D node cards

**Solution:** Convert to real `<button>` where semantically correct. Where truly a click-to-select card, add `role="button"` + `tabIndex={0}` + `onKeyDown` for Enter/Space.

Create `useButtonish` hook:
```jsx
// desktop/cortex-control-center/src/hooks/useButtonish.js
export function useButtonish(onClick, disabled) {
  return {
    role: 'button',
    tabIndex: disabled ? -1 : 0,
    'aria-disabled': disabled || undefined,
    onClick: disabled ? undefined : onClick,
    onKeyDown: disabled ? undefined : (e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        onClick?.(e);
      }
    },
  };
}
```

**Files:**
- `desktop/cortex-control-center/src/hooks/useButtonish.js` *(new)*
- `App.jsx` — replace 5 `div onClick` patterns (lines above) with `<button>` or `useButtonish`
- `BrainVisualizer.jsx:314` — same treatment on node cards

**Verification:**
- Tab-walk test: every interactive element reachable in logical order
- Screen reader announces each as "button" with name
- Enter + Space both activate

### A.3 — Dialog focus trap + ARIA (~6 hrs)

**Problem:** 3 dialogs (Connection Settings `App.jsx:4061`, Editor Setup `App.jsx:4125`, Metric Legend `App.jsx:4370`) lack `aria-modal`, focus trap, initial focus, focus restoration, Escape handler.

**Solution:** Migrate to Radix Dialog. Radix handles all 5 concerns out of the box.

```jsx
import * as Dialog from '@radix-ui/react-dialog';

<Dialog.Root open={open} onOpenChange={setOpen}>
  <Dialog.Portal>
    <Dialog.Overlay className="dialog-overlay" />
    <Dialog.Content className="dialog-content" aria-describedby="dialog-description">
      <Dialog.Title>Connection Settings</Dialog.Title>
      <Dialog.Description id="dialog-description">
        Configure how Cortex connects to the local daemon.
      </Dialog.Description>
      {/* form fields */}
      <Dialog.Close asChild>
        <button>Cancel</button>
      </Dialog.Close>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>
```

**Files:**
- `desktop/cortex-control-center/package.json` — add `@radix-ui/react-dialog`
- `App.jsx:4061-4200` — rewrite Connection dialog
- `App.jsx:4125-4200` — rewrite Editor Setup dialog
- `App.jsx:4370-4415` — rewrite Metric Legend dialog
- `desktop/cortex-control-center/src/styles.css` — port `.connection-overlay` + `.connection-dialog` styles to Radix `data-state` attributes

**Verification:**
- Dialog opens → focus lands on first tabbable element
- Tab cycles within dialog
- Escape closes dialog
- Close → focus returns to trigger
- Screen reader announces dialog role + title + description

### A.4 — Form labels (~2 hrs)

**Problem:** 9 form inputs placeholder-only. Placeholder disappears on focus, invisible to screen readers.

**Locations (from audit):**
- `App.jsx:4165-4188` — Connection Host / Port / Auth Token
- `App.jsx:894-897` — Task Title
- `App.jsx:1003-1006` — Task Completion Summary
- `App.jsx:4891-4898` — Memory Search
- `App.jsx:1267, 1280` — Conflict selects
- `App.jsx:4563, 4810, 4824` — Agent filter selects

**Solution:** Every input gets a visible `<label htmlFor>`. Visually-hidden label only as last resort.

```jsx
<div className="field">
  <label htmlFor="conn-host">Host</label>
  <input
    id="conn-host"
    type="text"
    placeholder="127.0.0.1"
    value={host}
    onChange={(e) => setHost(e.target.value)}
  />
</div>
```

**Files:**
- `App.jsx` — 9 field conversions
- `styles.css` — `.field` layout (label above input, consistent spacing)
- `sr-only.css` *(new if not present)* — visually-hidden utility class for edge cases

**Verification:**
- axe-core: no `label-missing` violations
- NVDA: announces field label on focus
- Click on label focuses input

### A.5 — Live region for async feedback (~2 hrs)

**Problem:** `setFeedbackMessage()` called 100+ times across App.jsx; no `aria-live` region. Screen reader users don't hear outcomes of their actions.

**Solution:** Custom `useAnnouncer` hook + single `<LiveRegion>` component mounted at App root.

```jsx
// desktop/cortex-control-center/src/hooks/useAnnouncer.js
import { createContext, useContext, useState, useCallback } from 'react';

const AnnouncerContext = createContext(null);

export function AnnouncerProvider({ children }) {
  const [polite, setPolite] = useState('');
  const [assertive, setAssertive] = useState('');

  const announce = useCallback((message, urgency = 'polite') => {
    if (urgency === 'assertive') {
      setAssertive('');
      setTimeout(() => setAssertive(message), 50);
    } else {
      setPolite('');
      setTimeout(() => setPolite(message), 50);
    }
  }, []);

  return (
    <AnnouncerContext.Provider value={announce}>
      {children}
      <div role="status" aria-live="polite" className="sr-only">{polite}</div>
      <div role="alert" aria-live="assertive" className="sr-only">{assertive}</div>
    </AnnouncerContext.Provider>
  );
}

export const useAnnouncer = () => useContext(AnnouncerContext);
```

**Files:**
- `desktop/cortex-control-center/src/hooks/useAnnouncer.js` *(new)*
- `App.jsx` — wrap app in `<AnnouncerProvider>`; replace `setFeedbackMessage()` calls with `announce(msg)`
- `styles.css` — `.sr-only` utility if not present

**Verification:**
- Store/delete/connect actions announced
- Errors announced as `assertive`, successes as `polite`
- No double-announcements from rapid clicks (key is the `''` reset + 50ms delay)

### A.6 — Reduced-motion runtime plumbing (~4 hrs)

**Problem:** Single CSS `@media (prefers-reduced-motion: reduce)` rule covers ~6 classes. 24+ animations (Brain auto-rotate, Analytics projections, pulse dots, shimmer, list-enter) ignore user preference.

**Solution:** Two-layer strategy.

**Layer 1 — CSS tokens:**
```css
:root {
  --duration-short: 150ms;
  --duration-medium: 220ms;
  --duration-long: 320ms;
  --ease-standard: cubic-bezier(0.2, 0, 0, 1);
  --ease-emphasized: cubic-bezier(0.3, 0, 0, 1);
  --ease-in: cubic-bezier(0.4, 0, 1, 1);
  --ease-out: cubic-bezier(0, 0, 0.2, 1);
}

@media (prefers-reduced-motion: reduce) {
  :root {
    --duration-short: 0ms;
    --duration-medium: 0ms;
    --duration-long: 0ms;
  }
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
    scroll-behavior: auto !important;
  }
}
```

All existing `transition: ... 200ms` instances migrate to `transition: ... var(--duration-medium)`.

**Layer 2 — React hook:**
```js
// desktop/cortex-control-center/src/hooks/useReducedMotion.js
import { useSyncExternalStore } from 'react';

const query = '(prefers-reduced-motion: reduce)';

function subscribe(callback) {
  const mql = window.matchMedia(query);
  mql.addEventListener('change', callback);
  return () => mql.removeEventListener('change', callback);
}

function getSnapshot() {
  return window.matchMedia(query).matches;
}

function getServerSnapshot() {
  return false;
}

export function useReducedMotion() {
  return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
```

`BrainVisualizer.jsx` auto-rotate:
```jsx
const reducedMotion = useReducedMotion();

useEffect(() => {
  if (reducedMotion) return; // no auto-rotate
  const id = requestAnimationFrame(tick);
  return () => cancelAnimationFrame(id);
}, [reducedMotion]);
```

**Files:**
- `desktop/cortex-control-center/src/design/tokens/motion.css` *(new)* — tokens + reduced-motion override
- `desktop/cortex-control-center/src/hooks/useReducedMotion.js` *(new)*
- `BrainVisualizer.jsx` — gate auto-rotate
- `App.jsx` — Analytics projections at line 469, check before animationDelay
- `styles.css` — migrate 30+ transition rules to use tokens

**Verification:**
- Set OS "Reduce motion" preference → every animation stops or completes instantly
- Brain auto-rotate stops
- Pulse dots become static
- Skip-to-panel still transitions (state change; keep opacity-only per motion research — "Reduced ≠ no-motion")

### A.7 — Target size audit (~2 hrs, WCAG 2.5.8)

**Problem:** Some interactive elements may be < 24×24 (WCAG 2.2 AA minimum). We're targeting 32×32 (above AA, matches macOS HIG).

**Solution:** CSS audit + fixes.

```css
button, [role="button"], a, input[type="checkbox"], input[type="radio"] {
  min-width: 32px;
  min-height: 32px;
}
```

Add exceptions for inline links in text (per 2.5.8 exception clause).

**Files:**
- `styles.css` — global min-size rule + targeted exception selectors
- Visual audit across all screens

**Verification:**
- axe-core `target-size` rule: no violations
- Hit-target plugin in Chrome DevTools

### A.8 — Initial ESLint + axe-core CI wiring (~4 hrs)

**Problem:** No a11y lint or test coverage today.

**Solution:** Install + configure lint + minimal axe-core smoke in CI.

```bash
npm install --save-dev eslint-plugin-jsx-a11y vitest-axe @axe-core/react
```

`eslint.config.js` (flat config):
```js
import jsxA11y from 'eslint-plugin-jsx-a11y';

export default [
  jsxA11y.flatConfigs.strict,
  // ... existing config
];
```

`src/App.jsx` — dev-only axe:
```jsx
if (import.meta.env.DEV) {
  import('@axe-core/react').then(({ default: axe }) => {
    axe(React, ReactDOM, 1000);
  });
}
```

`vitest.config.js` — add `vitest-axe` matchers; first a11y test in `src/__tests__/a11y-smoke.test.jsx`.

CI job update in `.github/workflows/ci.yml` — run `npm run lint` + `npm test -- a11y`.

**Files:**
- `desktop/cortex-control-center/eslint.config.js` — flat config with jsx-a11y/strict
- `desktop/cortex-control-center/package.json` — new dev deps
- `desktop/cortex-control-center/src/App.jsx` — dev-mode axe import
- `desktop/cortex-control-center/src/__tests__/a11y-smoke.test.jsx` *(new)* — renders main flows, asserts no axe violations
- `.github/workflows/ci.yml` — desktop-a11y step

**Verification:**
- Lint catches new `div onClick` without keyboard handlers
- Dev-mode axe logs violations to console
- CI a11y smoke green

### Sprint A acceptance gate

- [ ] `cargo test` + `cargo clippy -D warnings` still green (no daemon touches)
- [ ] `npm --prefix desktop/cortex-control-center test` green + includes a11y-smoke test
- [ ] `npm --prefix desktop/cortex-control-center run lint` zero jsx-a11y errors
- [ ] Manual Tab-walk on Overview, Memory, Brain, Analytics, Work, Agents — every interactive element reachable with visible focus
- [ ] Connection dialog: opens with focus trapped, Escape closes, focus returns
- [ ] Setting OS reduced-motion preference → all animations stop/shorten
- [ ] NVDA announces form labels on focus
- [ ] NVDA announces action outcomes via live region

---

## 4. Sprint B — Settings panel (Weeks 3-4, ~40 hrs)

First-class user-facing chrome for all accessibility + motion + connection + keyboard preferences.

### B.0 — Budgets backend handoff from C3 (landed 2026-05-05)

C3 backend landed in `b41f7be` and gives U1 a read contract for a Settings/Budgets section. Do not rebuild backend enforcement in the UI; consume health/admin data and treat write support as a follow-up workflow.

Health JSON example:

```json
{
  "budgets": {
    "configLoaded": true,
    "enabled": true,
    "source": "C:\\Users\\aditya\\.cortex\\budgets.toml",
    "error": null,
    "endpoints": {
      "store": { "limit": 120, "windowSeconds": 60, "window_seconds": 60 },
      "recall": { "limit": 300, "windowSeconds": 60, "window_seconds": 60 },
      "boot": { "limit": 60, "windowSeconds": 60, "window_seconds": 60 },
      "mcp": { "limit": 240, "windowSeconds": 60, "window_seconds": 60 }
    },
    "recentDenials": 0,
    "recent_denials": 0
  }
}
```

Admin JSON shape:

```bash
cortex admin budgets status --json
cortex admin budgets validate --path C:\Users\aditya\.cortex\budgets.toml --json
```

Both commands return the same `budgets` object shape as `/health.budgets` without live denial counters beyond `0` for local status/validate output.

Fields needed by UI:
- `configLoaded` / `config_loaded` — missing file means "unlimited / not configured", not an error.
- `enabled` — false when missing, disabled by config, or invalid.
- `source` — show the local config path.
- `error.code`, `error.message`, `error.endpoint`, `error.field` — render invalid config state.
- `endpoints.{store,recall,boot,mcp}.limit` and `.windowSeconds` — render configured rows; missing endpoint means unlimited.
- `recentDenials` — non-zero exhausted/recent denial indicator.

Required UI states:
- Empty/missing config: `configLoaded=false`, `enabled=false`, `error=null`, empty endpoints. Show unlimited/default-off state.
- Disabled config: `configLoaded=true`, `enabled=false`, `error=null`, endpoints may still be present. Show "configured but paused".
- Invalid config: `error` object present. Show fail-closed state and validation message; do not present it as partially active.
- Active config: `enabled=true`, endpoints map populated only for configured endpoint sections. Missing endpoint rows are unlimited.
- Exhausted/recent denial: `recentDenials > 0`. Show recent rejection state; detailed per-endpoint usage history is not available in this backend slice.

Remaining backend/UI gap:
- C3 did not ship a daemon write endpoint for editing budgets. U1 can either write `budgets.toml` through a local app/file capability or keep edits as local drafts and call `cortex admin budgets validate --path <draft>` before asking the operator to apply.
- The backend currently enforces call-count windows only. Token budgets, daily per-agent budgets, and durable usage history are deferred.

### B.1 — Settings panel skeleton (~8 hrs)

**Navigation entry:** add `Settings` to primary sidebar (below existing nav items, above `About`).

**Layout:** left sidebar (sections) + right content area. Radix Tabs with `activationMode="manual"` (users must Enter on a tab to activate — expected for settings).

**Files:**
- `desktop/cortex-control-center/src/settings/SettingsPanel.jsx` *(new)*
- `desktop/cortex-control-center/src/settings/SettingsSidebar.jsx` *(new)*
- `App.jsx` — add route/panel entry
- `styles.css` — settings-specific layout

**Sections:**
1. **Accessibility** (B.2)
2. **Appearance & Motion** (B.3)
3. **Connection** (B.4 — move existing Connection dialog content here; keep dialog for quick-access overflow)
4. **Keyboard & Navigation** (B.5)

### B.2 — Accessibility section (~8 hrs)

Controls:
- **Reduced motion** — radio: `System` / `On` / `Off` (default `System`)
- **Higher contrast** — toggle: `On` / `Off` (default `Off`); bumps `:root` custom properties
- **Larger text** — slider: 100% / 110% / 120% / 130% (default 100%); affects `html { font-size }`
- **Focus highlight strength** — radio: `Standard` / `Enhanced` (default `Standard`)

Persistence: localStorage via settings context. Optional daemon sync via new `/settings` endpoint (defer daemon side to v0.7.0; local-only for v0.6.0).

**Files:**
- `desktop/cortex-control-center/src/settings/sections/Accessibility.jsx` *(new)*
- `desktop/cortex-control-center/src/settings/SettingsContext.jsx` *(new)* — useReducer, localStorage persistence
- `desktop/cortex-control-center/src/hooks/useReducedMotion.js` — enhance to respect 3-state override (see below)

**Extend `useReducedMotion`:**
```js
export function useReducedMotion() {
  const osPreference = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
  const { reducedMotion } = useSettings(); // 'system' | 'on' | 'off'

  if (reducedMotion === 'on') return true;
  if (reducedMotion === 'off') return false;
  return osPreference; // 'system'
}
```

### B.3 — Appearance & Motion section (~6 hrs)

Controls:
- **Theme** — radio: `Dark` (default, current) / `High Contrast Dark` / `Light` (deferred to v0.7.0 — show as coming-soon)
- **Motion level** — radio: `Full` / `Subtle` / `Minimal` (default `Full`)
  - `Subtle` — kills translate/scale, keeps opacity (motion research recommendation)
  - `Minimal` — same as reduced-motion=On
- **Density** — radio: `Comfortable` / `Compact` (default `Comfortable`)

**Files:**
- `desktop/cortex-control-center/src/settings/sections/AppearanceMotion.jsx` *(new)*
- `design/tokens/motion.css` — wire motion level to token overrides

### B.4 — Connection section (~6 hrs)

Move host / port / auth token fields from dialog into Settings. Keep existing quick-access dialog (same form, different mount point) — pattern used by most modern apps (Slack, VS Code, etc.).

Prefill logic for WCAG 3.3.7 Redundant Entry — remember last successful connection, prefill on next attempt.

**Files:**
- `desktop/cortex-control-center/src/settings/sections/Connection.jsx` *(new)*
- `App.jsx` — refactor Connection dialog to share component with Settings section

### B.5 — Keyboard & Navigation section (~6 hrs)

Controls:
- **Shortcuts reference** — static table of registered keyboard shortcuts (currently: sidebar collapse, panel nav if any)
- **Shortcut customization** — `Coming in v0.7.0` placeholder (out of scope)
- **Tab order preview** — optional debug feature, toggleable

Include **Help shortcut** (`F1` or `?`) that opens this section — satisfies WCAG 3.2.6 Consistent Help.

**Files:**
- `desktop/cortex-control-center/src/settings/sections/KeyboardNav.jsx` *(new)*
- `App.jsx` — register global `F1` handler
- `desktop/cortex-control-center/src/data/shortcuts.json` *(new)* — canonical shortcut list

### B.6 — Settings persistence (~6 hrs)

`SettingsContext` reducer with:
- `loadSettings()` — reads localStorage on mount, falls back to defaults
- `setSetting(key, value)` — writes immediately, debounced ~100ms for numeric/slider values
- `resetDefaults()` — clears localStorage, restores defaults
- `exportSettings()` / `importSettings()` — JSON round-trip for backup (power-user feature)

Shape:
```json
{
  "version": 1,
  "accessibility": {
    "reducedMotion": "system",
    "higherContrast": false,
    "largerText": 100,
    "focusHighlight": "standard"
  },
  "appearance": {
    "theme": "dark",
    "motionLevel": "full",
    "density": "comfortable"
  },
  "connection": {
    "host": "127.0.0.1",
    "port": 7437,
    "lastSuccessful": { "host": "127.0.0.1", "port": 7437, "at": "2026-04-23T..." }
  },
  "keyboard": {
    "helpShortcut": "F1"
  }
}
```

Schema versioning for future migrations.

**Files:**
- `desktop/cortex-control-center/src/settings/SettingsContext.jsx`
- `desktop/cortex-control-center/src/settings/schema.js` — version-aware loader

### Sprint B acceptance gate

- [ ] Settings panel accessible from primary nav
- [ ] All 4 sub-sections render, each section navigable by keyboard + screen reader
- [ ] Changes persist across app restart
- [ ] Reduced-motion override wins over OS pref when set
- [ ] Connection section and dialog share the same form component
- [ ] F1 opens Keyboard & Navigation section
- [ ] axe-core clean on Settings panel
- [ ] Settings round-trip: export → clear → import → identical state

---

## 5. Sprint C — Motion system (Weeks 5-6, ~40 hrs)

Unified token-driven motion + library adoption. Sprint A laid the token foundation; Sprint C applies them everywhere.

### C.1 — Motion token consolidation (~6 hrs)

Replace all scattered `transition: ... 200ms` and `animation: ... 300ms` literals with tokens from `design/tokens/motion.css`.

grep target: `\b(transition|animation)(-duration)?:\s*[\w()-]+?\s+\d+ms`

**Files:**
- `desktop/cortex-control-center/src/styles.css` — bulk-migrate 30+ rules
- `desktop/cortex-control-center/src/live-surface.css` — same
- `BrainVisualizer.jsx` inline styles — same

Token scale:
| Token | Value | Use |
|-------|-------|-----|
| `--duration-short` | 150ms | State changes, hover, focus |
| `--duration-medium` | 220ms | Panel transitions, nav, tabs |
| `--duration-long` | 320ms | Route changes, modal entry/exit |
| `--ease-standard` | `cubic-bezier(0.2, 0, 0, 1)` | Default |
| `--ease-emphasized` | `cubic-bezier(0.3, 0, 0, 1)` | Large movements |
| `--ease-in` | `cubic-bezier(0.4, 0, 1, 1)` | Exits |
| `--ease-out` | `cubic-bezier(0, 0, 0.2, 1)` | Entrances |
| `--ease-linear` | `linear` | Skeleton shimmer, progress |
| `--ease-spring` | `cubic-bezier(0.5, -0.3, 0.5, 1.3)` | Delightful accents (sparingly) |

### C.2 — Sidebar collapse animation (~6 hrs)

**Current:** ad-hoc width change + label flicker.
**Target:** unified `220ms` width transition + opacity-stagger label fade (`80ms` delay on expand, immediate on collapse).

Canonical widths:
- Expanded: `260px` (middle of 240-300 research range)
- Collapsed: `72px` (middle of 48-80 research range, leaves room for icon + safe target area)

**Files:**
- `styles.css` — `.app-sidebar`, `.nav-item`, `.nav-item__label` rules
- `App.jsx` — remove any duplicated collapse/expand logic

### C.3 — Tab/panel transition system (~8 hrs)

**Current:** hard snap between panels.
**Target:** `120ms` cross-fade per motion research recommendation for distinct data views (Memory / Brain / Analytics have different content, not spatial continuity like photo gallery).

Use `motion/react` `AnimatePresence` + `m.div`:
```jsx
import { m, AnimatePresence, LazyMotion, domAnimation } from 'motion/react';

<LazyMotion features={domAnimation}>
  <AnimatePresence mode="wait">
    <m.div
      key={activePanel}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.12 }}
    >
      {renderPanel(activePanel)}
    </m.div>
  </AnimatePresence>
</LazyMotion>
```

Reduced-motion: `motion/react` respects `prefers-reduced-motion` automatically via `useReducedMotion` — but our hook already wires in. Pass `transition={{ duration: reducedMotion ? 0 : 0.12 }}`.

**Files:**
- `desktop/cortex-control-center/package.json` — add `motion`
- `App.jsx` — wrap main content area in LazyMotion + AnimatePresence

### C.4 — List/card enter animations via auto-animate (~4 hrs)

Memory list, Feed, Tasks — currently use staggered CSS keyframes (`list-enter`, `card-enter`). Replace with `@formkit/auto-animate` — auto-applies smooth transitions on list mutations, auto-respects reduced-motion.

```jsx
import { useAutoAnimate } from '@formkit/auto-animate/react';

function MemoryList({ items }) {
  const [parent] = useAutoAnimate();
  return (
    <ul ref={parent}>
      {items.map((m) => <li key={m.id}>{m.text}</li>)}
    </ul>
  );
}
```

Delete `@keyframes list-enter`, `@keyframes card-enter`, `@keyframes metric-enter` from styles.css.

**Files:**
- `package.json` — add `@formkit/auto-animate`
- Memory list, Feed list, Task columns, Metric grid — add `useAutoAnimate`
- `styles.css` — remove keyframes

### C.5 — Pulse / shimmer / decorative animations (~6 hrs)

**Audit list (from research):**
- `@keyframes pulse-dot` (line 222) — online status
- `@keyframes pulse-starting-dot` (line 227) — starting state
- `@keyframes pulse-dot-cyan` (line 838)
- `@keyframes pulse-text` (line 927)
- `@keyframes shimmer` (line 1492) — loading state
- `@keyframes draw-line` (line 1451) — sparkline
- `@keyframes projection-trace` (line 2038) — Analytics

**Rule:** Every `@keyframes` rule gets a `@media (prefers-reduced-motion: reduce)` counterpart that either:
- Makes it static (pulse dots → solid color)
- Makes it instant (draw-line → full-width immediately)
- Keeps opacity transitions (per motion research "keep opacity as semantic signal")

**Files:**
- `styles.css` — audit all `@keyframes`, add reduced-motion variants

### C.6 — BrainVisualizer motion (~6 hrs)

**Current:** continuous auto-rotate in 3D view; drag-to-pan.
**Target:**
- Auto-rotate gated by `useReducedMotion`
- Drag alternative: keyboard arrows move selected node (satisfies WCAG 2.5.7 Dragging Movements)
- Arrow keys + Enter/Space to select nodes
- 2D fallback view gets same keyboard treatment

**Files:**
- `BrainVisualizer.jsx` — add keyboard handlers, useReducedMotion gate
- Brain 2D node card — useButtonish hook from Sprint A.2

### C.7 — Toast animations via Sonner (~4 hrs)

Currently: ad-hoc feedback messages. Replace with Sonner:
```jsx
import { Toaster, toast } from 'sonner';

// root App
<Toaster position="bottom-right" richColors closeButton />

// anywhere
toast.success('Connected to daemon');
toast.error('Connection failed: port 7437 refused');
```

Sonner ships correct `aria-live` + keyboard-dismiss + respects reduced-motion by default.

**Files:**
- `package.json` — add `sonner`
- `App.jsx` — mount `<Toaster>`
- Replace `setFeedbackMessage()` sites (that weren't just for announcer) with `toast.*`

### Sprint C acceptance gate

- [ ] Zero hardcoded duration literals in CSS (grep: `\d{2,4}ms` in `desktop/cortex-control-center/src/` only matches token defs)
- [ ] Sidebar collapse uses tokens; single width transition
- [ ] Panel switch uses `motion/react` cross-fade; 120ms
- [ ] Lists use auto-animate; reduced-motion kills on-mutation animation
- [ ] All `@keyframes` have reduced-motion counterparts
- [ ] BrainVisualizer keyboard-operable
- [ ] Sonner toasts with aria-live announce state changes
- [ ] Bundle size delta ≤ 45kB gzip (budget check)
- [ ] FPS sustained at 60 during sidebar collapse, tab switch, list re-order (Chrome DevTools Performance capture)

---

## 6. Sprint D — Testing infrastructure (Weeks 7-8, ~40 hrs)

### D.1 — vitest-axe unit suite (~8 hrs)

Per-component a11y assertions. One test per Settings sub-section, dialog, form, list.

Example:
```jsx
// src/__tests__/settings.test.jsx
import { render } from '@testing-library/react';
import { axe } from 'vitest-axe';

test('Settings panel has no a11y violations', async () => {
  const { container } = render(<SettingsPanel />);
  const results = await axe(container);
  expect(results).toHaveNoViolations();
});
```

**Gotcha from WCAG research:** `happy-dom` has known incompatibilities with axe. Use `jsdom` for a11y tests, even if rest of suite uses happy-dom. Wire via `vitest.config.js` environment override per file.

**Files:**
- `vitest.config.js` — dual environment setup
- `src/__tests__/a11y-*.test.jsx` — one per top-level component

**Target coverage:** every render-intensive component has an a11y test. ~15-20 tests.

### D.2 — Playwright E2E matrix (~12 hrs)

Install `@axe-core/playwright`. Run full a11y audit against:
- WebView2 (Windows)
- WKWebView (macOS)
- WebKitGTK (Linux)

Playwright tests:
- `e2e/a11y-keyboard.spec.js` — Tab-walk every flow
- `e2e/a11y-screen-reader-semantics.spec.js` — assert role/name/value on critical elements
- `e2e/a11y-axe-full.spec.js` — full WCAG 2.2 AA scan per route
- `e2e/a11y-reduced-motion.spec.js` — simulate preference, verify no animation beyond 0.01ms

CI job matrix: 3 platforms × 4 specs = 12 runs.

**Files:**
- `desktop/cortex-control-center/playwright.config.js`
- `desktop/cortex-control-center/e2e/a11y-*.spec.js`
- `.github/workflows/ci.yml` — new job `a11y-e2e-matrix`

### D.3 — Manual screen reader walkthroughs (~12 hrs)

Per WCAG research: NVDA+Firefox primary, VoiceOver+Safari secondary, Narrator+Edge smoke.

**Walkthrough script** (canonical, same across all SR pairs):
1. App launch — is "Cortex Control Center" announced with landmarks?
2. Tab through sidebar — each nav item announced with label + state
3. Activate Memory panel — landmark + heading + count announced
4. Open Memory search — input label announced
5. Type and submit search — results list announced with count
6. Open Brain panel — switch 2D/3D tabs via keyboard — tab state announced
7. Select a Brain node — node name + details announced
8. Open Settings — sections navigable by arrow keys in sidebar
9. Change reduced-motion preference — announced immediately (live region)
10. Open Connection dialog — focus trap + title + description announced
11. Submit with bad port — error announced via assertive live region
12. Cancel dialog — focus returns to trigger

**Output:** per-SR walkthrough report committed to `docs/internal/v060/a11y-walkthroughs/<sr>-<date>.md`.

**Files:**
- `docs/internal/v060/a11y-walkthroughs/nvda-firefox-<DATE>.md`
- `docs/internal/v060/a11y-walkthroughs/voiceover-safari-<DATE>.md`
- `docs/internal/v060/a11y-walkthroughs/narrator-edge-<DATE>.md`
- `docs/internal/v060/a11y-walkthroughs/README.md` — shared walkthrough script template

### D.4 — Contrast + non-text contrast pass (~4 hrs)

Run axe-core contrast rule across all main views. Spot-check fails with WebAIM Contrast Checker.

From audit: most pass AAA, but disabled button states + inactive tab styles untested. Fix any AA gaps.

**Files:**
- `styles.css` — adjusted disabled/inactive tokens
- Audit report in `docs/internal/v060/a11y-walkthroughs/contrast-<DATE>.md`

### D.5 — Integrated a11y CI gate (~4 hrs)

Combine all automated gates into single CI job:
- lint (jsx-a11y strict)
- vitest-axe unit
- Playwright axe E2E matrix

Fail the build on any critical/serious violation.

**Files:**
- `.github/workflows/ci.yml` — consolidated `accessibility` job

### Sprint D acceptance gate

- [ ] vitest-axe suite: zero violations on all tested components
- [ ] Playwright E2E: zero violations on all 3 platforms × 4 specs
- [ ] NVDA+Firefox walkthrough: all 12 steps pass, report committed
- [ ] VoiceOver+Safari walkthrough: all 12 steps pass (allow 1-2 known-safari quirks if documented)
- [ ] Narrator+Edge smoke: 12 steps pass
- [ ] Contrast audit: all main views pass WCAG AA
- [ ] CI accessibility job blocking on failure

---

## 7. Sprint E — Polish (Week 9, ~20 hrs)

Catch-all for residual issues found during testing.

- Close remaining axe violations (expected count: 5-15 after Sprint D)
- ARIA completeness audit — tablists have `role="tab"` + `aria-selected`; expandables have `aria-expanded`; lists have `role="list"`
- Zoom/reflow test at 200% zoom, 375×812 viewport
- Documentation: `Info/accessibility.md` — public-facing a11y statement
- CHANGELOG entries consolidated

### Sprint E acceptance gate

- [ ] Zero axe critical/serious violations anywhere in app
- [ ] 200% zoom usable without horizontal scroll
- [ ] `Info/accessibility.md` written
- [ ] CHANGELOG drafted

---

## 8. File touch map (consolidated from sprints)

### New files (30)

```
desktop/cortex-control-center/src/hooks/
  useButtonish.js
  useAnnouncer.js
  useReducedMotion.js

desktop/cortex-control-center/src/settings/
  SettingsContext.jsx
  SettingsPanel.jsx
  SettingsSidebar.jsx
  schema.js
  sections/
    Accessibility.jsx
    AppearanceMotion.jsx
    Connection.jsx
    KeyboardNav.jsx

desktop/cortex-control-center/src/design/tokens/
  focus.css
  motion.css

desktop/cortex-control-center/src/data/
  shortcuts.json

desktop/cortex-control-center/src/__tests__/
  a11y-smoke.test.jsx
  a11y-settings.test.jsx
  a11y-dialogs.test.jsx
  a11y-forms.test.jsx
  (~12 more)

desktop/cortex-control-center/e2e/
  a11y-keyboard.spec.js
  a11y-screen-reader-semantics.spec.js
  a11y-axe-full.spec.js
  a11y-reduced-motion.spec.js
  playwright.config.js

docs/internal/v060/a11y-walkthroughs/
  README.md
  nvda-firefox-<DATE>.md
  voiceover-safari-<DATE>.md
  narrator-edge-<DATE>.md
  contrast-<DATE>.md

Info/accessibility.md  (public)

desktop/cortex-control-center/eslint.config.js  (if not present)
```

### Modified files (~8)

```
desktop/cortex-control-center/
  package.json              # new deps
  vitest.config.js          # a11y env
  src/App.jsx               # dialogs, live region, keyboard, settings nav
  src/BrainVisualizer.jsx   # keyboard, reduced-motion
  src/styles.css            # token migration, focus ring, contrast, reduced-motion
  src/live-surface.css      # token migration
  src/index.html            # any meta/skip-link additions

.github/workflows/ci.yml    # a11y jobs
```

---

## 9. Dependency additions

```json
{
  "dependencies": {
    "@radix-ui/react-dialog": "^1.x",
    "@radix-ui/react-tabs": "^1.x",
    "@radix-ui/react-toast": "^1.x",
    "react-focus-lock": "^2.x",
    "sonner": "^1.x",
    "motion": "^11.x",
    "@formkit/auto-animate": "^0.9.x",
    "react-hook-form": "^7.x",
    "downshift": "^9.x"
  },
  "devDependencies": {
    "@axe-core/react": "^4.x",
    "@axe-core/playwright": "^4.x",
    "vitest-axe": "^0.1.x",
    "eslint-plugin-jsx-a11y": "^6.10.2",
    "@playwright/test": "^1.x"
  }
}
```

Total gzipped impact: ~45kB (bundle) + dev-only tooling. Tauri desktop OK with this. If web mode re-enables later, audit again.

---

## 10. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Radix Dialog styling conflicts with existing CSS | Medium | Medium | Scope Dialog rewrite to one dialog first (Metric Legend — simplest); validate approach before rewriting Connection + Editor Setup |
| `motion/react` + Radix both want to control exit animations | Low | Low | Use Radix's built-in data-state CSS transitions (not Motion) for Dialog — documented pattern in Radix docs |
| axe-core false positives burn dev time | Medium | Low | Use axe `rules` config to pin known-good ignores; document each in `.axe-ignore.md` with rationale |
| `happy-dom` breaks vitest-axe silently (known) | High | Low | Use `jsdom` environment for a11y test files explicitly; WCAG research flagged this |
| Screen reader walkthroughs surface bugs not in axe | Certain | Medium | That's the point. Budget Sprint E for fixes. |
| Bundle size hits soft limit | Low | Low | LazyMotion loads domAnimation on-demand; can tree-shake further if needed |
| Tauri WebView2 / WKWebView / WebKitGTK parity issues with `:focus-visible`, `inert`, `:has()` | Medium | Medium | E2E matrix catches these; fall back to JS polyfills only where blocking |
| React 19 + Radix compatibility issues | Low | Low | Radix has shipped React 19 support; verify latest versions before lock |
| Sprint A overruns → Sprints B-E compressed | Medium | High | Strict Sprint A acceptance gate; if slipping, cut Sprint C scope (keep token system, drop auto-animate + Sonner for v0.6.1) |

---

## 11. Non-goals

Explicitly NOT in v0.6.0:

- **Mobile responsive** (web mode / phone viewport) — desktop app only for v0.6.0
- **JAWS testing** — v0.7.0
- **Light theme** — v0.7.0 (dark only for v0.6.0)
- **Keyboard shortcut customization** — v0.7.0 (show defaults only)
- **Internationalization / RTL** — v0.8.0+
- **High-contrast forced-colors mode** (Windows HCM) — v0.7.0
- **Voice control** — out of scope (handled by OS, our job is to not fight it)
- **WCAG 2.2 AAA** — not a target; spot-fixes if trivial
- **APCA contrast algorithm** — WCAG 3 still Working Draft; stick with WCAG 2.x math
- **Daemon-side settings sync** — local-only for v0.6.0; `/settings` endpoint v0.7.0

---

## 12. Release acceptance criteria (final)

All of these must hold at v0.6.0 cut (2026-07-16):

- [ ] `npm run lint` — zero `jsx-a11y` violations
- [ ] `npm test` — all vitest-axe assertions green
- [ ] CI `a11y-e2e-matrix` job — green on WebView2 + WKWebView + WebKitGTK
- [ ] Manual walkthrough reports committed for NVDA+Firefox, VoiceOver+Safari, Narrator+Edge
- [ ] axe-core dev-mode console shows zero critical/serious violations when app runs
- [ ] Settings panel functional with 4 sections, persists across restart
- [ ] Reduced-motion 3-state (System/On/Off) honored at runtime
- [ ] All dialogs use Radix or equivalent focus-trap pattern
- [ ] All form inputs have visible `<label>` elements
- [ ] All interactive elements ≥ 32×32
- [ ] All `@keyframes` have reduced-motion counterparts
- [ ] Focus rings visible on every interactive element (`:focus-visible`)
- [ ] F1 opens Keyboard & Navigation section (Consistent Help)
- [ ] Connection prefill honors last successful (Redundant Entry)
- [ ] `Info/accessibility.md` published

---

## 13. Sprint summary

| Sprint | Weeks | Hours | Primary deliverable |
|--------|-------|-------|---------------------|
| A — Critical remediation | 2 | 40 | Focus, keyboard, dialogs, labels, live region, reduced-motion foundation |
| B — Settings panel | 3-4 | 40 | First-class Settings with 4 sections + persistence |
| C — Motion system | 5-6 | 40 | Token-driven motion, library adoption, unified transitions |
| D — Testing infrastructure | 7-8 | 40 | vitest-axe + Playwright E2E + manual walkthroughs |
| E — Polish | 9 | 20 | Residual fixes, contrast pass, docs |
| **Total** | **8 weeks** | **180 hrs** | **WCAG 2.2 AA compliance** |

Fits Tier 1 headline budget. Leaves Sprint C slack for compressed Foundation Carryovers work running in parallel track.

---

## 14. Cross-references

- Research: `research/a11y-codebase-audit.md`, `research/a11y-wcag22-research.md`, `research/a11y-motion-research.md`, `research/a11y-react-libs.md`
- Scope lock: `scope-lock.md`
- Sibling plans: `foundation-carryovers.md`, `governance-economics.md`
- Status tracker: `unified-status-plan.md` §3B
- Release queue: `updates-to-readme.md` §1.1-1.3
- Changelog: `comprehensive-changelog.md` entries 2A-2N
