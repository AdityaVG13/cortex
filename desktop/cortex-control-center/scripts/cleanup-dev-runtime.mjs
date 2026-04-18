import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectDir = resolve(scriptDir, "..");

function escapeForPowerShell(value) {
  return String(value).replace(/'/g, "''");
}

export function cleanupDevRuntime({ quiet = false } = {}) {
  if (process.platform !== "win32") {
    return { cleaned: false, reason: "non-windows" };
  }

  const psProject = escapeForPowerShell(projectDir);
  const psSelfPid = Number(process.pid) || 0;
  const psScript = `
$project = '${psProject}'
$selfPid = ${psSelfPid}
$killed = @()
$errors = @()

$procMatches = Get-CimInstance Win32_Process | Where-Object {
  $_.ProcessId -ne $selfPid -and (
    ($_.Name -eq 'node.exe' -and $_.CommandLine -and $_.CommandLine.ToLower().Contains($project.ToLower())) -or
    ($_.Name -eq 'cortex-control-center.exe' -and $_.ExecutablePath -and $_.ExecutablePath.ToLower().Contains($project.ToLower()))
  )
}

foreach ($proc in $procMatches) {
  try {
    Stop-Process -Id $proc.ProcessId -Force -ErrorAction Stop
    $killed += ('{0}:{1}' -f $proc.Name, $proc.ProcessId)
  } catch {
    $errors += ('{0}:{1}' -f $proc.ProcessId, $_.Exception.Message)
  }
}

$listeners = Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue
foreach ($entry in $listeners) {
  if ($entry.OwningProcess -eq $selfPid) { continue }
  try {
    Stop-Process -Id $entry.OwningProcess -Force -ErrorAction Stop
    $killed += ('port1420:{0}' -f $entry.OwningProcess)
  } catch {
    $errors += ('port1420:{0}:{1}' -f $entry.OwningProcess, $_.Exception.Message)
  }
}

$result = [pscustomobject]@{
  killed = $killed
  errors = $errors
}
$result | ConvertTo-Json -Compress
`;

  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", psScript],
    {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    },
  );

  if (result.error) {
    if (!quiet) {
      console.warn(`[dev-cleanup] failed to run cleanup: ${result.error.message}`);
    }
    return { cleaned: false, reason: result.error.message };
  }

  if (!quiet && result.stderr?.trim()) {
    console.warn(`[dev-cleanup] ${result.stderr.trim()}`);
  }

  let payload = { killed: [], errors: [] };
  try {
    payload = JSON.parse(result.stdout?.trim() || "{\"killed\":[],\"errors\":[]}");
  } catch {
    // Keep defaults if parsing fails.
  }

  if (!quiet && payload.killed.length) {
    console.log(`[dev-cleanup] removed stale processes: ${payload.killed.join(", ")}`);
  }
  if (!quiet && payload.errors.length) {
    console.warn(`[dev-cleanup] cleanup warnings: ${payload.errors.join("; ")}`);
  }

  return {
    cleaned: payload.killed.length > 0,
    killed: payload.killed,
    errors: payload.errors,
  };
}

const invokedUrl = process.argv[1] ? pathToFileURL(process.argv[1]).href : "";
if (invokedUrl && import.meta.url === invokedUrl) {
  cleanupDevRuntime();
}
