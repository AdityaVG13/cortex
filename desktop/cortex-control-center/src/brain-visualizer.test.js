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

  it("uses native graph nodes for smooth type-colored rendering", () => {
    expect(source).not.toContain("import * as THREE");
    expect(source).not.toContain("nodeThreeObject={");
    expect(source).toContain("const BRAIN_NODE_COLORS = Object.freeze");
    expect(source).toContain("nodeColor={resolveNodeColor}");
    expect(source).toContain("nodeVal={resolveNodeValue}");
    expect(source).toContain("nodeResolution={8}");
  });

  it("keeps the Jarvis-style Brain overlay static and cheap", () => {
    expect(source).toContain("brain-orbital-ring brain-orbital-ring-a");
    expect(source).toContain("brain-orbital-ring brain-orbital-ring-b");
    expect(source).not.toContain("brain-scanline");

    expect(readBlock(css, ".brain-orbital-ring {")).not.toContain("animation:");
    expect(css).not.toContain("brain-scanline");
    expect(css).not.toContain("brain-ring-drift");
    expect(css).not.toContain(".brain-container canvas");
  });
});
