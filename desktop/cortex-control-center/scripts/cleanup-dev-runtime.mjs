import{spawnSync}from"node:child_process";import{dirname,resolve}from"node:path";import{fileURLToPath,pathToFileURL}from"node:url";const scriptDir=dirname(fileURLToPath(import.meta.url)),projectDir=resolve(scriptDir,"..");function escapeForPowerShell(value){return String(value).replace(/'/g,"''")}function cleanupDevRuntime({quiet=!1}={}){if(process.platform!=="win32")return{cleaned:!1,reason:"non-windows"};const psProject=escapeForPowerShell(projectDir),psSelfPid=Number(process.pid)||0,psScript=`
$project = '${psProject}'
$selfPid = ${psSelfPid}
$killed = @()
$cleanedSessions = @()
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

$runtimeRoots = @()
if ($env:USERPROFILE) {
  $runtimeRoots += (Join-Path $env:USERPROFILE '.cortex\\runtime\\control-center-dev')
}
if ($env:HOME) {
  $homeRuntime = Join-Path $env:HOME '.cortex\\runtime\\control-center-dev'
  if ($runtimeRoots -notcontains $homeRuntime) {
    $runtimeRoots += $homeRuntime
  }
}

foreach ($runtimeRoot in $runtimeRoots) {
  if (-not (Test-Path $runtimeRoot)) { continue }
  $sessionDirs = Get-ChildItem -Path $runtimeRoot -Directory -Filter 'session-*' -ErrorAction SilentlyContinue
  foreach ($sessionDir in $sessionDirs) {
    $sessionPath = $sessionDir.FullName
    $sessionProcs = Get-CimInstance Win32_Process | Where-Object {
      $_.ProcessId -ne $selfPid -and
      $_.ExecutablePath -and
      $_.ExecutablePath.ToLower().StartsWith($sessionPath.ToLower())
    }
    foreach ($proc in $sessionProcs) {
      try {
        Stop-Process -Id $proc.ProcessId -Force -ErrorAction Stop
        $killed += ('{0}:{1}' -f $proc.Name, $proc.ProcessId)
      } catch {
        $errors += ('session-proc:{0}:{1}' -f $proc.ProcessId, $_.Exception.Message)
      }
    }
    try {
      Remove-Item -LiteralPath $sessionPath -Recurse -Force -ErrorAction Stop
      $cleanedSessions += $sessionPath
    } catch {
      $errors += ('session-dir:{0}:{1}' -f $sessionPath, $_.Exception.Message)
    }
  }
}

$result = [pscustomobject]@{
  killed = $killed
  cleanedSessions = $cleanedSessions
  errors = $errors
}
$result | ConvertTo-Json -Compress
`,result=spawnSync("powershell.exe",["-NoProfile","-ExecutionPolicy","Bypass","-Command",psScript],{encoding:"utf8",stdio:["ignore","pipe","pipe"],windowsHide:!0});if(result.error)return quiet||console.warn(`[dev-cleanup] failed to run cleanup: ${result.error.message}`),{cleaned:!1,reason:result.error.message};!quiet&&result.stderr?.trim()&&console.warn(`[dev-cleanup] ${result.stderr.trim()}`);let payload={killed:[],cleanedSessions:[],errors:[]};try{payload=JSON.parse(result.stdout?.trim()||'{"killed":[],"cleanedSessions":[],"errors":[]}')}catch{}return!quiet&&payload.killed.length&&console.log(`[dev-cleanup] removed stale processes: ${payload.killed.join(", ")}`),!quiet&&payload.cleanedSessions.length&&console.log(`[dev-cleanup] removed stale session wrappers: ${payload.cleanedSessions.length}`),!quiet&&payload.errors.length&&console.warn(`[dev-cleanup] cleanup warnings: ${payload.errors.join("; ")}`),{cleaned:payload.killed.length>0||payload.cleanedSessions.length>0,killed:payload.killed,cleanedSessions:payload.cleanedSessions,errors:payload.errors}}const invokedUrl=process.argv[1]?pathToFileURL(process.argv[1]).href:"";invokedUrl&&import.meta.url===invokedUrl&&cleanupDevRuntime();export{cleanupDevRuntime};
