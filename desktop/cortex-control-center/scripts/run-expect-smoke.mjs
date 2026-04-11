import { spawn } from "node:child_process";
import { once } from "node:events";
import net from "node:net";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import { startMockCortexServer } from "./mock-cortex-server.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectDir = resolve(scriptDir, "..");
const nodeCommand = process.execPath;
const cliArgs = process.argv.slice(2);
const viteBin = resolve(projectDir, "node_modules", "vite", "bin", "vite.js");
const expectCliBin = resolve(projectDir, "node_modules", "expect-cli", "dist", "index.js");
const previewHost = process.env.EXPECT_SMOKE_HOST || "127.0.0.1";
const requestedPreviewPort = Number(process.env.EXPECT_SMOKE_PORT || "4173");
const requestedApiPort = Number(process.env.EXPECT_SMOKE_API_PORT || "7438");
const token = process.env.EXPECT_SMOKE_TOKEN || "expect-smoke-token";
const forcedHeaded = cliArgs.includes("--headed") || process.env.EXPECT_SMOKE_HEADED === "true";
const isCi = cliArgs.includes("--ci") || !forcedHeaded;
const agent = process.env.EXPECT_CLI_AGENT || "";
const expectTarget = process.env.EXPECT_CLI_TARGET || (process.env.CI === "true" ? "changes" : "unstaged");

function readCliOption(name) {
  const direct = cliArgs.find((arg) => arg.startsWith(`${name}=`));
  if (direct) {
    return direct.slice(name.length + 1);
  }

  const index = cliArgs.indexOf(name);
  if (index >= 0) {
    return cliArgs[index + 1] || "";
  }

  return "";
}

const requestedScope = (
  readCliOption("--scope") ||
  (cliArgs.includes("--work-only") ? "work" : "") ||
  (cliArgs.includes("--overview-only") ? "overview" : "") ||
  process.env.EXPECT_SMOKE_SCOPE ||
  "all"
).toLowerCase();

if (!["all", "overview", "work"].includes(requestedScope)) {
  throw new Error(`Unsupported smoke scope: ${requestedScope}`);
}

const defaultMessage = [
  "Open the Cortex Control Center smoke URL and verify the app loads without authentication errors.",
  "Open the Analytics tab and verify the Monte Carlo Savings Horizon card renders with a fixed summary box whose p10, p50, and p90 labels do not overlap the baseline marker.",
  "Open the Brain tab and verify the primary HUD headline says Neural topology and does not overlap the lower control strip.",
  "Fail on runtime errors, auth warnings, broken navigation, or overlapping HUD/chart labels. Report failures only.",
].join(" ");

const defaultWorkMessage = [
  "Open the Cortex Control Center Work surface and verify the operator selector is visible and actionable.",
  "Select operator Codex, claim the pending task titled 'Wire operator actions into the Work surface', then complete it with a short summary.",
  "Unlock the Codex file lock for desktop/cortex-control-center/src/App.jsx.",
  "Send a short message from Codex to Claude from the Work surface composer.",
  "Enable unread feed filtering if available, then acknowledge the visible feed entries.",
  "Fail on runtime errors, missing controls, auth warnings, or actions that do not update the UI state. Report failures only.",
].join(" ");

function wait(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForHttp(url, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url, { method: "GET" });
      if (response.ok) return;
    } catch {
      // server is not ready yet
    }
    await wait(500);
  }
  throw new Error(`Timed out waiting for ${url}`);
}

async function reservePort(host, preferredPort) {
  const tryListen = (port) =>
    new Promise((resolve, reject) => {
      const server = net.createServer();
      server.unref();
      server.once("error", (error) => {
        server.close();
        if (error?.code === "EADDRINUSE") {
          resolve(null);
          return;
        }
        reject(error);
      });
      server.listen(port, host, () => {
        const address = server.address();
        const reservedPort = typeof address === "object" && address ? address.port : preferredPort;
        server.close((error) => (error ? reject(error) : resolve(reservedPort)));
      });
    });

  const preferred = await tryListen(preferredPort);
  if (preferred) return preferred;
  const ephemeral = await tryListen(0);
  if (ephemeral) return ephemeral;
  throw new Error(`Could not reserve a local port for ${host}`);
}

function spawnProcess(command, args, label) {
  const child = spawn(command, args, {
    cwd: projectDir,
    env: {
      ...process.env,
      FORCE_COLOR: process.env.FORCE_COLOR || "1",
    },
    stdio: "inherit",
    windowsHide: true,
  });

  child.once("error", (error) => {
    console.error(`[expect-smoke] ${label} failed to start: ${error.message}`);
  });

  return child;
}

async function stopChild(child, label) {
  if (!child || child.killed || child.exitCode !== null) return;
  child.kill("SIGTERM");
  const exited = Promise.race([
    once(child, "exit"),
    wait(5_000).then(() => null),
  ]);
  const result = await exited;
  if (result === null && child.exitCode === null) {
    child.kill("SIGKILL");
    await once(child, "exit").catch(() => {});
  }
  if (label) {
    console.log(`[expect-smoke] stopped ${label}`);
  }
}

const cleanupStack = [];

async function withCleanup(fn) {
  try {
    await fn();
  } finally {
    while (cleanupStack.length) {
      const cleanup = cleanupStack.pop();
      try {
        // eslint-disable-next-line no-await-in-loop
        await cleanup();
      } catch (error) {
        console.error(`[expect-smoke] cleanup failed: ${error.message}`);
      }
    }
  }
}

async function runExpectCheck({ smokeUrl, message, agent, isCi }) {
  const expectArgs = [
    expectCliBin,
    "--url",
    smokeUrl.toString(),
    "--target",
    expectTarget,
    "--message",
    message,
    "--yes",
    "--no-cookies",
    "--timeout",
    process.env.EXPECT_CLI_TIMEOUT || "600000",
  ];

  if (agent) {
    expectArgs.push("--agent", agent);
  }
  if (isCi) {
    expectArgs.push("--ci");
  }
  if (process.env.EXPECT_CLI_VERBOSE === "true") {
    expectArgs.push("--verbose");
  }

  console.log(`[expect-smoke] running expect-cli against ${smokeUrl.toString()}`);
  const expectProcess = spawnProcess(nodeCommand, expectArgs, "expect-cli");
  const [exitCode] = await once(expectProcess, "exit");
  if (exitCode !== 0) {
    throw new Error(`expect-cli exited with code ${exitCode}`);
  }
}

await withCleanup(async () => {
  const apiPort = await reservePort(previewHost, requestedApiPort);
  const previewPort = await reservePort(previewHost, requestedPreviewPort);

  const mockServer = await startMockCortexServer({
    host: previewHost,
    port: apiPort,
    token,
  });
  cleanupStack.push(async () => {
    await mockServer.close();
    console.log("[expect-smoke] stopped mock Cortex");
  });
  console.log(`[expect-smoke] mock Cortex ready at ${mockServer.baseUrl}`);

  const previewProcess = spawnProcess(
    nodeCommand,
    [viteBin, "preview", "--host", previewHost, "--port", String(previewPort), "--strictPort"],
    "vite preview",
  );
  cleanupStack.push(() => stopChild(previewProcess, "vite preview"));

  await waitForHttp(`http://${previewHost}:${previewPort}`);

  const makeSmokeUrl = (panel) => {
    const url = new URL(`http://${previewHost}:${previewPort}/`);
    url.searchParams.set("cortexBase", mockServer.baseUrl);
    url.searchParams.set("authToken", token);
    if (panel) {
      url.searchParams.set("panel", panel);
    }
    return url;
  };

  const smokeChecks = [];
  if (requestedScope !== "work") {
    smokeChecks.push({
      smokeUrl: makeSmokeUrl("overview"),
      message: process.env.EXPECT_SMOKE_MESSAGE || defaultMessage,
    });
  }
  if (requestedScope !== "overview") {
    smokeChecks.push({
      smokeUrl: makeSmokeUrl("work"),
      message: process.env.EXPECT_SMOKE_WORK_MESSAGE || defaultWorkMessage,
    });
  }

  for (const check of smokeChecks) {
    // eslint-disable-next-line no-await-in-loop
    await runExpectCheck({ ...check, agent, isCi });
  }
});
