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

describe("responsive reflow layout", () => {
  it("keeps the app shell shrinkable for zoomed narrow viewports", () => {
    expect(declarationsFor(css, ".app {")).toMatchObject({
      "grid-template-columns": "var(--sidebar-w) minmax(0, 1fr)",
      height: "100dvh",
      "min-width": "0",
    });

    expect(declarationsFor(css, ".app.sidebar-collapsed {")).toMatchObject({
      "grid-template-columns": "var(--sidebar-collapsed-w) minmax(0, 1fr)",
    });

    expect(declarationsFor(css, ".content {")).toMatchObject({
      "min-width": "0",
      "overflow-x": "hidden",
      "scrollbar-gutter": "stable",
    });

    expect(declarationsFor(css, ".panel-header-actions {")).toMatchObject({
      display: "flex",
      "justify-content": "flex-end",
      "flex-wrap": "wrap",
      "min-width": "0",
    });

    expect(declarationsFor(css, ".about-content {")).toMatchObject({
      "box-sizing": "border-box",
      width: "min(100%, 760px)",
    });

    expect(declarationsFor(css, ".about-heading {")).toMatchObject({
      "min-width": "0",
    });
  });

  it("keeps the skip link offscreen until keyboard focus reveals it", () => {
    expect(declarationsFor(css, ".skip-link {")).toMatchObject({
      position: "fixed",
      transform: "translateY(calc(-100% - 20px))",
      "font-size": "1rem",
    });

    expect(declarationsFor(css, ".skip-link:focus,")).toMatchObject({
      transform: "translateY(0)",
      outline: "2px solid var(--cyan-bright)",
    });
  });

  it("declares a 375px-safe mobile reflow breakpoint", () => {
    const mobile = readBlock(css, "@media (max-width: 480px)");

    expect(declarationsFor(mobile, ".sidebar {")).toMatchObject({
      "overflow-y": "auto",
      "overflow-x": "hidden",
    });

    expect(declarationsFor(mobile, ".topbar {")).toMatchObject({
      "max-height": "none",
      overflow: "visible",
    });

    expect(declarationsFor(mobile, ".sidebar.collapsed .nav-item,")).toMatchObject({
      "min-height": "44px",
    });

    expect(declarationsFor(mobile, 'input:not([type="checkbox"]),')).toMatchObject({
      "min-height": "44px",
    });

    expect(mobile).toMatch(
      /\.connection-dialog-close\s*\{[^}]*min-width:\s*44px;[^}]*width:\s*44px;[^}]*height:\s*44px;/,
    );

    expect(declarationsFor(mobile, ".overview-metrics,")).toMatchObject({
      "grid-template-columns": "minmax(0, 1fr)",
    });

    expect(declarationsFor(mobile, ".about-stats-grid,")).toMatchObject({
      "grid-template-columns": "minmax(0, 1fr)",
    });

    expect(declarationsFor(mobile, ".analytics-view-toggle {")).toMatchObject({
      display: "flex",
      "flex-direction": "column",
      width: "100%",
    });

    expect(declarationsFor(mobile, ".connection-actions,")).toMatchObject({
      "flex-direction": "column",
      width: "100%",
    });

    expect(declarationsFor(mobile, ".panel-header-actions .btn-sm,")).toMatchObject({
      width: "100%",
    });

    expect(declarationsFor(mobile, ".analytics-view-toggle .btn-sm,")).toMatchObject({
      width: "100%",
    });

    expect(declarationsFor(mobile, ".settings-shortcut-grid {")).toMatchObject({
      "grid-template-columns": "minmax(0, 1fr)",
    });

    expect(declarationsFor(mobile, ".about-content {")).toMatchObject({
      padding: "1rem",
    });

    expect(declarationsFor(mobile, ".about-contributor {")).toMatchObject({
      display: "grid",
      "grid-template-columns": "auto minmax(0, 1fr)",
    });

    expect(declarationsFor(mobile, ".about-contributor-role {")).toMatchObject({
      "grid-column": "2",
      "margin-left": "0",
      "text-align": "left",
    });

    expect(declarationsFor(mobile, ".brain-panel {")).toMatchObject({
      margin: "-16px -10px 0",
    });

    expect(declarationsFor(mobile, ".brain-detail,")).toMatchObject({
      left: "10px",
      right: "10px",
      width: "auto",
      "max-width": "none",
    });
  });
});
