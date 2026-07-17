import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SRC = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const CSS_FILES = [
  "styles/base.css",
  "styles/layout.css",
  "styles/components.css",
  "styles/topbar.css",
  "styles/animations.css",
  "styles/charts.css",
  "styles/panels/analytics.css",
  "styles/panels/brain.css",
  "styles/overrides-2026-a.css",
  "styles/overrides-2026-b.css",
  "styles/sidebar-collapse.css",
  "styles/connection-dialog.css",
  "styles/panels/conflicts.css",
  "styles/accessibility.css",
];

export function readBundledStyles() {
  return CSS_FILES.map((rel) => fs.readFileSync(path.join(SRC, rel), "utf8")).join("\n");
}
