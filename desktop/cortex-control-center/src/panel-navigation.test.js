import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const appSource = readFileSync(new URL("./App.jsx", import.meta.url), "utf8");

function readBlock(source, needle) {
  const start = source.indexOf(needle);
  expect(start, `missing source block ${needle}`).toBeGreaterThanOrEqual(0);

  const bodyStart = source.indexOf("{", start);
  expect(bodyStart, `missing source body for ${needle}`).toBeGreaterThanOrEqual(0);

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

  throw new Error(`unterminated source block ${needle}`);
}

describe("panel navigation scheduling", () => {
  it("updates the active panel urgently after recording motion direction", () => {
    const changePanel = readBlock(appSource, "const changePanel = useCallback");

    expect(changePanel).toContain("setPanelMotionDirection(");
    expect(changePanel).toContain("setPanel(nextPanel);");
    expect(changePanel).not.toContain("startTransition(() => setPanel(nextPanel))");
  });

  it("keeps the settings panel mounted while inactive", () => {
    expect(appSource).toContain(
      'className={`panel settings-panel ${panel === "settings" ? "active" : "panel-hidden"}`}',
    );
    expect(appSource).toContain('aria-hidden={panel === "settings" ? undefined : true}');
  });

  it("exposes a keyboard skip link to the main content landmark", () => {
    const skipLinkIndex = appSource.indexOf('<a className="skip-link" href="#main-content">');
    const sidebarIndex = appSource.indexOf("<aside");
    const mainIndex = appSource.indexOf('<main id="main-content" className="content" tabIndex={-1}>');

    expect(skipLinkIndex, "missing skip link").toBeGreaterThanOrEqual(0);
    expect(mainIndex, "missing skip target main landmark").toBeGreaterThanOrEqual(0);
    expect(skipLinkIndex, "skip link should be the first focusable shell control").toBeLessThan(sidebarIndex);
  });

  it("gives placeholder-only task and permission controls accessible names", () => {
    expect(appSource).toContain('aria-label={`Completion summary for ${task.title}`}');
    expect(appSource).toContain('aria-label="Client id for permission grant"');
    expect(appSource).toContain(': "Operator message body"');
  });

  it("announces budget validation and load errors as alerts", () => {
    expect(appSource).toContain(
      '{budgetSummary.error ? <p className="settings-error" role="alert">{budgetSummary.error}</p> : null}',
    );
    expect(appSource).toContain(
      '{budgetDraftError ? <p className="settings-error" role="alert">{budgetDraftError}</p> : null}',
    );
  });

  it("does not load desktop budget state during the settings panel entry animation", () => {
    expect(appSource).toContain("const budgetReloadTimer = window.setTimeout(() => {");
    expect(appSource).toContain("}, effectiveReducedMotion ? 0 : MOTION_MS.panel);");
    expect(appSource).toContain("window.clearTimeout(budgetReloadTimer);");
  });
});
