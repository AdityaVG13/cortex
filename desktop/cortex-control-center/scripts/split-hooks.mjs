#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC = path.join(ROOT, "src");
const hookPath = path.join(SRC, "app/hooks/useDashboardHooks.js");
const lines = fs.readFileSync(hookPath, "utf8").split("\n");

const imports = lines.slice(0, 111).join("\n");
const body = lines.slice(112, -1); // drop closing }

const HOOK_RETURN = fs.readFileSync(path.join(ROOT, "scripts/split-panels-and-hooks.mjs"), "utf8")
  .match(/const HOOK_RETURN_KEYS = `([\s\S]*?)`;/)[1];

function sliceBody(start, end) {
  return body.slice(start, end).join("\n");
}

const stateBody = sliceBody(0, 805 - 113);
const refreshBody = sliceBody(805 - 113, 1506 - 113);
const effectsBody = sliceBody(1508 - 113, 2530 - 113);
const handlersBody = sliceBody(2531 - 113, 2947 - 113);

function writeHook(name, fnName, param, extraImport, block, ret) {
  const content = `${imports}
${extraImport}
export function ${fnName}(${param}) {
${block}
${ret}
}
`;
  fs.writeFileSync(path.join(SRC, `app/hooks/${name}`), content);
}

writeHook(
  "useDashboardState.js",
  "useDashboardState",
  "",
  "",
  stateBody,
  `  return {
    ${HOOK_RETURN},
  };`,
);

writeHook(
  "useRefreshOrchestration.js",
  "useRefreshOrchestration",
  "ctx",
  "",
  refreshBody.replace(/^  /gm, "  "),
  `  return {
    ...ctx,
    ${HOOK_RETURN},
  };`,
);

writeHook(
  "useSseStream.js",
  "useSseStream",
  "ctx",
  "",
  sliceBody(1755 - 113, 1870 - 113),
  "  return ctx;",
);

writeHook(
  "useDashboardEffects.js",
  "useDashboardEffects",
  "ctx",
  "",
  effectsBody.replace(/  useEffect\(\(\) => \{\n    let stream = null;[\s\S]*?  \}, \[cortexBase, refreshAllRef\]\);\n\n/m, ""),
  "  return ctx;",
);

writeHook(
  "useDashboardHandlers.js",
  "useDashboardHandlers",
  "ctx",
  "",
  handlersBody,
  `  return {
    ...ctx,
    ${HOOK_RETURN},
  };`,
);

const composer = `import { useDashboardState } from "./useDashboardState.js";
import { useRefreshOrchestration } from "./useRefreshOrchestration.js";
import { useSseStream } from "./useSseStream.js";
import { useDashboardEffects } from "./useDashboardEffects.js";
import { useDashboardHandlers } from "./useDashboardHandlers.js";

export function useDashboardHooks() {
  let ctx = useDashboardState();
  ctx = useRefreshOrchestration(ctx);
  ctx = useSseStream(ctx);
  ctx = useDashboardEffects(ctx);
  return useDashboardHandlers(ctx);
}
`;

fs.writeFileSync(hookPath, composer);
console.log("Split useDashboardHooks into sub-hooks");
