import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

function readBlock(source, needle) {
  const start = source.indexOf(needle);
  expect(start, `missing CSS block ${needle}`).toBeGreaterThanOrEqual(0);

  const bodyStart = source.indexOf("{", start);
  expect(bodyStart, `missing CSS body for ${needle}`).toBeGreaterThanOrEqual(0);

  let depth = 1;
  for (let index = bodyStart + 1; index < source.length; index += 1) {
    if (source[index] === "{") {
      depth += 1;
    } else if (source[index] === "}") {
      depth -= 1;
    }

    if (depth === 0) {
      return source.slice(bodyStart + 1, index);
    }
  }

  throw new Error(`unterminated CSS block ${needle}`);
}

function declarationsFor(source, needle) {
  return Object.fromEntries(
    [...readBlock(source, needle).matchAll(/([\w-]+):\s*([^;]+);/g)].map(([, name, value]) => [
      name,
      value.trim(),
    ]),
  );
}

describe("panel transition system", () => {
  it("animates top-level panel changes with directional spatial motion", () => {
    expect(declarationsFor(css, ".panel {")).toMatchObject({
      "transform-origin": "top center",
      "will-change": "opacity, transform",
    });

    expect(declarationsFor(css, '.panel-stage[data-panel-direction="forward"] > .panel.active')).toMatchObject({
      animation: "panel-slide-forward var(--motion-panel) var(--motion-ease) both",
    });

    expect(declarationsFor(css, '.panel-stage[data-panel-direction="backward"] > .panel.active')).toMatchObject({
      animation: "panel-slide-backward var(--motion-panel) var(--motion-ease) both",
    });

    expect(readBlock(css, "@keyframes panel-slide-forward")).toContain(
      "transform: translate3d(18px, 0, 0)",
    );
    expect(readBlock(css, "@keyframes panel-slide-backward")).toContain(
      "transform: translate3d(-18px, 0, 0)",
    );
  });

  it("adds tab-panel motion without bypassing reduced-motion settings", () => {
    expect(declarationsFor(css, ".analytics-mode-panel {")).toMatchObject({
      animation: "tab-panel-enter var(--motion-panel) var(--motion-ease) both",
      "will-change": "opacity, transform",
    });

    expect(readBlock(css, "@keyframes tab-panel-enter")).toContain(
      "transform: translate3d(0, 8px, 0) scale(0.995)",
    );
    expect(css).toContain(':root[data-cortex-effective-reduced-motion="reduce"] .analytics-mode-panel');
    expect(css).toContain(':root:not([data-cortex-reduced-motion="full"]) .analytics-mode-panel');
  });
});
