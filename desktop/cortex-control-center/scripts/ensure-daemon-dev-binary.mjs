import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, statSync } from "node:fs";
import { dirname, extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectDir = resolve(scriptDir, "..");
const repoRoot = resolve(projectDir, "..", "..");
const daemonRoot = resolve(repoRoot, "daemon-rs");
const daemonManifestPath = resolve(repoRoot, "daemon-rs", "Cargo.toml");
const daemonLockPath = resolve(repoRoot, "daemon-rs", "Cargo.lock");
const daemonSrcPath = resolve(repoRoot, "daemon-rs", "src");
const daemonTargetDir = resolve(repoRoot, "daemon-rs", "target-control-center-dev");
const daemonBinary = process.platform === "win32"
  ? resolve(daemonTargetDir, "debug", "cortex.exe")
  : resolve(daemonTargetDir, "debug", "cortex");

const WATCHED_EXTENSIONS = new Set([".rs", ".toml", ".lock"]);
const IGNORED_DIR_NAMES = new Set([
  "target",
  "target-control-center-dev",
  "target-control-center-release",
  ".tmp",
  "builds",
]);

function runCargoBuild() {
  const command = process.platform === "win32" ? "cargo.exe" : "cargo";
  const args = [
    "build",
    "--target-dir",
    daemonTargetDir,
    "--manifest-path",
    daemonManifestPath,
  ];
  const invokeBuild = () => spawnSync(command, args, {
    cwd: projectDir,
    stdio: "pipe",
    windowsHide: true,
    encoding: "utf8",
  });
  const emitBuildOutput = (result) => {
    if (result.stdout) process.stdout.write(result.stdout);
    if (result.stderr) process.stderr.write(result.stderr);
  };
  const isWindowsDevBinaryLockError = (result) => {
    if (process.platform !== "win32") return false;
    const text = `${result.stdout || ""}\n${result.stderr || ""}`.toLowerCase();
    return (
      text.includes("failed to remove file") &&
      text.includes("target-control-center-dev") &&
      text.includes("cortex.exe") &&
      text.includes("access is denied")
    );
  };
  const stopConflictingDevDaemons = () => {
    if (process.platform !== "win32") return;
    const repoRootLiteral = repoRoot.replace(/'/g, "''");
    const daemonPathLiteral = daemonBinary.replace(/'/g, "''");
    const sharedDebugLiteral = resolve(repoRoot, "daemon-rs", "target", "debug", "cortex.exe")
      .replace(/'/g, "''");
    const runtimeRootLiteral = resolve(
      process.env.USERPROFILE || process.env.HOME || "",
      ".cortex",
      "runtime",
      "control-center-dev",
    ).replace(/'/g, "''");
    const script = [
      `$repoRoot='${repoRootLiteral}'.ToLower()`,
      `$target='${daemonPathLiteral}'`,
      `$sharedDebug='${sharedDebugLiteral}'.ToLower()`,
      `$runtimeRoot='${runtimeRootLiteral}'.ToLower()`,
      "$matches = Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath -ieq $target }",
      "$stale = Get-CimInstance Win32_Process | Where-Object {",
      "  if (-not $_.ExecutablePath) { return $false }",
      "  $exe = $_.ExecutablePath.ToLower()",
      "  ($exe.StartsWith($repoRoot) -and $exe -eq $sharedDebug) -or",
      "  ($runtimeRoot -and $exe.StartsWith($runtimeRoot) -and [System.IO.Path]::GetFileName($exe).StartsWith('cortex-dev-run'))",
      "}",
      "foreach ($proc in $matches) {",
      "  Write-Host \"[ensure-daemon] stopping locked dev daemon pid=$($proc.ProcessId) path=$($proc.ExecutablePath)\"",
      "  Stop-Process -Id $proc.ProcessId -Force -ErrorAction SilentlyContinue",
      "}",
      "foreach ($proc in $stale) {",
      "  Write-Host \"[ensure-daemon] stopping stale conflicting daemon pid=$($proc.ProcessId) path=$($proc.ExecutablePath)\"",
      "  Stop-Process -Id $proc.ProcessId -Force -ErrorAction SilentlyContinue",
      "}",
    ].join("; ");
    const cleanupResult = spawnSync("powershell.exe", ["-NoProfile", "-Command", script], {
      cwd: projectDir,
      stdio: "pipe",
      encoding: "utf8",
      windowsHide: true,
    });
    if (cleanupResult.stdout) {
      process.stdout.write(cleanupResult.stdout);
    }
    if (cleanupResult.stderr) {
      process.stderr.write(cleanupResult.stderr);
    }
  };

  stopConflictingDevDaemons();
  let result = invokeBuild();
  emitBuildOutput(result);
  if (typeof result.status === "number" && result.status === 0) {
    return;
  }

  if (isWindowsDevBinaryLockError(result)) {
    console.warn("[ensure-daemon] dev daemon binary is locked; stopping old process and retrying build once");
    stopConflictingDevDaemons();
    result = invokeBuild();
    emitBuildOutput(result);
    if (typeof result.status === "number" && result.status === 0) {
      return;
    }
  }

  process.exit(result.status ?? 1);
}

function latestInputMtimeMs(path) {
  if (!existsSync(path)) return 0;
  const info = statSync(path);
  if (info.isFile()) return info.mtimeMs;
  if (!info.isDirectory()) return 0;

  let newest = 0;
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (IGNORED_DIR_NAMES.has(entry.name)) continue;
      newest = Math.max(newest, latestInputMtimeMs(resolve(path, entry.name)));
      continue;
    }
    if (!entry.isFile()) continue;
    if (!WATCHED_EXTENSIONS.has(extname(entry.name).toLowerCase())) continue;
    newest = Math.max(newest, statSync(resolve(path, entry.name)).mtimeMs);
  }
  return newest;
}

function shouldRebuild() {
  if (!existsSync(daemonBinary)) {
    return { rebuild: true, reason: "missing binary" };
  }
  const binaryMtime = statSync(daemonBinary).mtimeMs;
  const inputMtime = Math.max(
    latestInputMtimeMs(daemonManifestPath),
    latestInputMtimeMs(daemonLockPath),
    latestInputMtimeMs(daemonSrcPath),
  );
  if (inputMtime > binaryMtime) {
    return { rebuild: true, reason: "source newer than binary" };
  }
  return { rebuild: false, reason: "binary up to date" };
}

const decision = shouldRebuild();
if (decision.rebuild) {
  console.log(`[ensure-daemon] building dev daemon binary (${decision.reason}) at ${daemonBinary}`);
  runCargoBuild();
} else {
  console.log(`[ensure-daemon] using existing dev daemon binary (${decision.reason}) at ${daemonBinary}`);
}
