import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("./BrainVisualizer.jsx", import.meta.url), "utf8");
const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

function readBlock(text, needle) {
  const start = text.indexOf(needle);
  expect(start, `missing block ${needle}`).toBeGreaterThanOrEqual(0);

  const bodyStart = text.indexOf("{", start);
  expect(bodyStart, `missing body for ${needle}`).toBeGreaterThanOrEqual(0);

  let depth = 1;
  for (let index = bodyStart + 1; index < text.length; index += 1) {
    if (text[index] === "{") {
      depth += 1;
    } else if (text[index] === "}") {
      depth -= 1;
    }

    if (depth === 0) {
      return text.slice(bodyStart + 1, index);
    }
  }

  throw new Error(`unterminated block ${needle}`);
}

describe("Brain visualizer", () => {
  it("keeps selected-node recall flow visible in the graph and details panel", () => {
    expect(source).toContain("const selectedFlow = useMemo");
    expect(source).toContain("const isSelectedFlowLink = useCallback");
    expect(source).toContain("brain-stat brain-stat-flow");
    expect(source).toContain("brain-flow-panel");
    expect(source).toContain("linkDirectionalParticles={link => link.type === \"conflict\" ? 3 : isSelectedFlowLink(link) ? 2 : 0}");
  });

  it("renders the cinematic Brain overlay without bypassing reduced motion", () => {
    expect(source).toContain("brain-orbital-ring brain-orbital-ring-a");
    expect(source).toContain("brain-orbital-ring brain-orbital-ring-b");
    expect(source).toContain("brain-scanline");

    expect(readBlock(css, ".brain-orbital-ring {")).toContain("animation: brain-ring-drift 28s linear infinite");
    expect(css).toContain("animation: brain-scanline-drift 9s var(--motion-ease) infinite");
    expect(css).toContain(':root[data-cortex-effective-reduced-motion="reduce"] .brain-orbital-ring');
    expect(css).toContain(':root:not([data-cortex-reduced-motion="full"]) .brain-scanline');
  });
});
