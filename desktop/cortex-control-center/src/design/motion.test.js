import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { MOTION_CSS_VARS, MOTION_EASING, MOTION_MS, easeOutCubic } from "./motion.js";

const css = readFileSync(new URL("../styles.css", import.meta.url), "utf8");

function readRootTokens() {
  const rootStart = css.indexOf(":root {");
  expect(rootStart).toBeGreaterThanOrEqual(0);
  const bodyStart = css.indexOf("{", rootStart);
  const bodyEnd = css.indexOf("}", bodyStart);
  expect(bodyEnd).toBeGreaterThan(bodyStart);

  return Object.fromEntries(
    [...css.slice(bodyStart + 1, bodyEnd).matchAll(/(--[\w-]+):\s*([^;]+);/g)].map(
      ([, name, value]) => [name, value.trim()],
    ),
  );
}

describe("motion design tokens", () => {
  it("keeps JS and CSS motion tokens aligned", () => {
    const root = readRootTokens();

    expect(root["--motion-base"]).toBe(MOTION_CSS_VARS.base);
    expect(root["--motion-panel"]).toBe(MOTION_CSS_VARS.panel);
    expect(root["--motion-shell"]).toBe(MOTION_CSS_VARS.shell);
    expect(root["--motion-ease"]).toBe(MOTION_CSS_VARS.ease);
    expect(root["--sidebar-transition"]).toBe("var(--motion-shell) var(--motion-ease)");
  });

  it("uses one easing language for JS number animation", () => {
    expect(MOTION_MS).toMatchObject({
      base: 200,
      panel: 340,
      shell: 320,
      number: 600,
      numberSlow: 1000,
    });
    expect(MOTION_EASING.standard).toBe("cubic-bezier(0.22, 1, 0.36, 1)");
    expect(easeOutCubic(0)).toBe(0);
    expect(easeOutCubic(1)).toBe(1);
    expect(easeOutCubic(-1)).toBe(0);
    expect(easeOutCubic(2)).toBe(1);
    expect(easeOutCubic(0.5)).toBeCloseTo(0.875);
  });
});
