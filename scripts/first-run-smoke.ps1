param(
  [string]$CortexCommand = "cortex",
  [string]$Agent = "cortex-onboarding-smoke"
)

$ErrorActionPreference = "Stop"

function Invoke-CortexCli {
  param([string[]]$Arguments)

  $stdoutPath = [System.IO.Path]::GetTempFileName()
  $stderrPath = [System.IO.Path]::GetTempFileName()
  try {
    $process = Start-Process `
      -FilePath $CortexCommand `
      -ArgumentList $Arguments `
      -RedirectStandardOutput $stdoutPath `
      -RedirectStandardError $stderrPath `
      -WindowStyle Hidden `
      -Wait `
      -PassThru
    [pscustomobject]@{
      Code = $process.ExitCode
      Text = (Get-Content -LiteralPath $stdoutPath -Raw)
      ErrorText = (Get-Content -LiteralPath $stderrPath -Raw)
    }
  } finally {
    Remove-Item -LiteralPath $stdoutPath,$stderrPath -Force -ErrorAction SilentlyContinue
  }
}

$statusResult = Invoke-CortexCli -Arguments @("status", "--json")
$status = $null
try {
  $status = $statusResult.Text | ConvertFrom-Json
} catch {
  Write-Error "cortex status --json did not return parseable JSON. Stdout:`n$($statusResult.Text)`nStderr:`n$($statusResult.ErrorText)"
}

if ($status.status -ne "ready") {
  $next = if ($status.nextAction.label) { $status.nextAction.label } else { "Run cortex status --json and inspect repair." }
  Write-Host "Cortex smoke blocked: $($status.status)"
  Write-Host "Next action: $next"
  if ($status.repair.command) {
    Write-Host "Repair command: $($status.repair.command)"
  }
  exit 1
}

$tokenPath = [string]$status.runtime.tokenPath
if (-not (Test-Path -LiteralPath $tokenPath)) {
  Write-Error "Token path from status does not exist: $tokenPath"
}
$token = (Get-Content -LiteralPath $tokenPath -Raw).Trim()
if (-not $token) {
  Write-Error "Token path is empty: $tokenPath"
}

$baseUrl = ([string]$status.runtime.baseUrl).TrimEnd("/")
$stamp = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$decision = "Cortex first-run smoke stored at $stamp"
$query = "first-run smoke stored $stamp"

$headers = @{
  "Authorization" = "Bearer $token"
  "X-Cortex-Request" = "true"
  "X-Source-Agent" = $Agent
  "Content-Type" = "application/json"
}
$body = @{
  decision = $decision
  context = "Disposable onboarding smoke memory; safe to archive later."
  type = "memory"
  source_agent = $Agent
} | ConvertTo-Json -Depth 4

$store = Invoke-RestMethod -Method Post -Uri "$baseUrl/store" -Headers $headers -Body $body
if (-not $store) {
  Write-Error "Store returned an empty response."
}

$recallUri = "$baseUrl/recall?q=$([uri]::EscapeDataString($query))&k=3&budget=200"
$recall = Invoke-RestMethod -Method Get -Uri $recallUri -Headers $headers
$results = @($recall.results)
if ($results.Count -lt 1) {
  Write-Error "Recall returned no results for smoke query."
}

Write-Host "Cortex first-run smoke passed."
Write-Host "Stored: $decision"
Write-Host "Recalled results: $($results.Count)"
