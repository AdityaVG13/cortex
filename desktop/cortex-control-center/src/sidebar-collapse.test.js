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
    [...readBlock(source, needle).matchAll(/(--?[\w-]+|[\w-]+):\s*([^;]+);/g)].map(
      ([, name, value]) => [name, value.trim()],
    ),
  );
}

function count(pattern) {
  return [...css.matchAll(pattern)].length;
}

describe("sidebar collapse tokens", () => {
  it("uses one canonical collapsed width token", () => {
    const root = declarationsFor(css, ":root {");

    expect(root["--sidebar-w"]).toBe("240px");
    expect(root["--sidebar-collapsed-w"]).toBe("64px");
    expect(count(/--sidebar-collapsed-w:\s*64px;/g)).toBe(1);
    expect(count(/width:\s*var\(--sidebar-collapsed-w\);/g)).toBe(1);
  });

  it("shares the same transition token for app grid and sidebar width", () => {
    const root = declarationsFor(css, ":root {");
    const app = declarationsFor(css, ".app {");
    const sidebar = declarationsFor(css, ".sidebar {");
    const collapsedApp = declarationsFor(css, ".app.sidebar-collapsed {");
    const collapsedSidebar = declarationsFor(css, ".sidebar.collapsed {");

    expect(root["--sidebar-transition"]).toBe("var(--motion-shell) var(--motion-ease)");
    expect(app.transition).toBe("grid-template-columns var(--sidebar-transition)");
    expect(sidebar.transition).toContain("width var(--sidebar-transition)");
    expect(collapsedApp["grid-template-columns"]).toBe(
      "var(--sidebar-collapsed-w) minmax(0, 1fr)",
    );
    expect(collapsedSidebar.width).toBe("var(--sidebar-collapsed-w)");
  });
});
