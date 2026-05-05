import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

const TEXT_TOKENS = ["--text", "--text-2", "--text-3"];
const NON_TEXT_TOKENS = ["--border", "--border-subtle"];
const SURFACE_TOKENS = ["--bg", "--surface", "--surface-2", "--surface-3"];
const HIGH_CONTRAST_TOKENS = [
  "--bg",
  "--bg-deep",
  "--surface",
  "--surface-2",
  "--surface-3",
  "--border",
  "--border-subtle",
  "--text",
  "--text-2",
  "--text-3",
  "--cyan",
  "--cyan-bright",
  "--cyan-dim",
];

function readRuleBody(selectorNeedle) {
  const selectorIndex = css.indexOf(selectorNeedle);
  expect(selectorIndex, `missing CSS selector ${selectorNeedle}`).toBeGreaterThanOrEqual(0);

  const bodyStart = css.indexOf("{", selectorIndex);
  expect(bodyStart, `missing CSS rule body for ${selectorNeedle}`).toBeGreaterThanOrEqual(0);

  let depth = 1;
  for (let index = bodyStart + 1; index < css.length; index += 1) {
    if (css[index] === "{") {
      depth += 1;
    } else if (css[index] === "}") {
      depth -= 1;
    }

    if (depth === 0) {
      return css.slice(bodyStart + 1, index);
    }
  }

  throw new Error(`unterminated CSS rule body for ${selectorNeedle}`);
}

function readTokens(selectorNeedle) {
  return Object.fromEntries(
    [...readRuleBody(selectorNeedle).matchAll(/(--[\w-]+):\s*([^;]+);/g)].map(
      ([, name, value]) => [name, value.trim()],
    ),
  );
}

function hexToRgb(value) {
  const match = value.match(/^#([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i);
  expect(match, `${value} must be a 6-digit hex color`).toBeTruthy();

  return match.slice(1).map((channel) => Number.parseInt(channel, 16) / 255);
}

function toLinear(channel) {
  return channel <= 0.03928
    ? channel / 12.92
    : ((channel + 0.055) / 1.055) ** 2.4;
}

function luminance(color) {
  const [red, green, blue] = hexToRgb(color).map(toLinear);
  return red * 0.2126 + green * 0.7152 + blue * 0.0722;
}

function contrastRatio(foreground, background) {
  const foregroundLum = luminance(foreground);
  const backgroundLum = luminance(background);
  const lighter = Math.max(foregroundLum, backgroundLum);
  const darker = Math.min(foregroundLum, backgroundLum);

  return (lighter + 0.05) / (darker + 0.05);
}

function expectThemeContrast(themeName, tokens) {
  for (const foreground of TEXT_TOKENS) {
    for (const background of SURFACE_TOKENS) {
      expect(
        contrastRatio(tokens[foreground], tokens[background]),
        `${themeName} ${foreground} on ${background} must meet WCAG AA text contrast`,
      ).toBeGreaterThanOrEqual(4.5);
    }
  }

  for (const foreground of NON_TEXT_TOKENS) {
    for (const background of SURFACE_TOKENS) {
      expect(
        contrastRatio(tokens[foreground], tokens[background]),
        `${themeName} ${foreground} against ${background} must meet WCAG non-text contrast`,
      ).toBeGreaterThanOrEqual(3);
    }
  }
}

describe("contrast design tokens", () => {
  it("meet WCAG AA text and non-text contrast thresholds", () => {
    expectThemeContrast("standard", readTokens(":root"));
    expectThemeContrast("high contrast", readTokens(':root[data-cortex-contrast="high"]'));
  });

  it("keeps the OS high-contrast media override aligned with the explicit setting", () => {
    const highContrast = readTokens(':root[data-cortex-contrast="high"]');
    const osHighContrast = readTokens(':root[data-cortex-contrast="standard"],');

    for (const token of HIGH_CONTRAST_TOKENS) {
      expect(osHighContrast[token], `${token} should match the explicit high contrast setting`).toBe(
        highContrast[token],
      );
    }
  });
});
