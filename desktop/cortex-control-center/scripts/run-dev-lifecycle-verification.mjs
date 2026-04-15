import { spawn } from "node:child_process";
import { once } from "node:events";
import { access, readFile } from "node:fs/promises";
import { constants as fsConstants } from "node:fs";
import os from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectDir = resolve(scriptDir, "..");
const npmCommand = process.platform === "win32" ? (process.env.ComSpec || "cmd.exe") : "npm";
const reportPath = join(
  os.tmpdir(),
  `cortex-dev-restart-reconnect-${Date.now()}-${process.pid}.json`,
);
const timeoutMs = Number(process.env.CORTEX_DEV_VERIFY_TIMEOUT_MS || "240000");

function wait(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForReportFile(path, child, timeout) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`tauri dev exited before verification completed (code ${child.exitCode}).`);
    }
    try {
      await access(path, fsConstants.F_OK);
      return;
    } catch {
      await wait(500);
    }
  }
  throw new Error(`Timed out waiting for verification report at ${path}.`);
}

function spawnProcess(command, args, label, extraEnv = {}) {
  const child = spawn(command, args, {
    cwd: projectDir,
    env: {
      ...process.env,
      ...extraEnv,
      FORCE_COLOR: process.env.FORCE_COLOR || "1",
    },
    stdio: "inherit",
    windowsHide: true,
  });

  child.once("error", (error) => {
    console.error(`[dev-verify] ${label} failed to start: ${error.message}`);
  });

  return child;
}

async function stopChild(child, label) {
  if (!child || child.killed || child.exitCode !== null) return;
  child.kill("SIGTERM");
  const exited = Promise.race([
    once(child, "exit"),
    wait(10000).then(() => null),
  ]);
  const result = await exited;
  if (result === null && child.exitCode === null) {
    child.kill("SIGKILL");
    await once(child, "exit").catch(() => {});
  }
  console.log(`[dev-verify] stopped ${label}`);
}

function printSummary(report) {
  console.log(`[dev-verify] report: ${report.reportPath || reportPath}`);
  console.log(`[dev-verify] status: ${report.success ? "passed" : "failed"}`);
  if (report.agent) {
    console.log(`[dev-verify] agent: ${report.agent}`);
  }
  for (const step of Array.isArray(report.steps) ? report.steps : []) {
    console.log(`[dev-verify] step: ${step.name}`);
  }
  if (report.error) {
    console.error(`[dev-verify] error: ${report.error}`);
  }
}

const npmArgs = process.platform === "win32"
  ? ["/d", "/s", "/c", "npm run dev"]
  : ["run", "dev"];

const child = spawnProcess(
  npmCommand,
  npmArgs,
  "tauri dev",
  {
    CORTEX_DEV_VERIFY_REPORT_PATH: reportPath,
    VITE_CORTEX_DEV_VERIFY_RESTART: "1",
  },
);

try {
  await waitForReportFile(reportPath, child, timeoutMs);
  const report = JSON.parse(await readFile(reportPath, "utf8"));
  printSummary(report);
  if (!report.success) {
    throw new Error(report.error || "Restart/reconnect verification failed.");
  }
} finally {
  await stopChild(child, "tauri dev");
}
