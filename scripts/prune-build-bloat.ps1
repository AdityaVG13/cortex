param(
    [string]$Root = "C:\Users\aditya\cortex",
    [int]$DaysUnused = 7,
    [switch]$Apply,
    [switch]$IncludeBenchmarkArtifacts
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-DirSizeBytes {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return 0 }
    $sum = (Get-ChildItem -LiteralPath $Path -Recurse -File -Force -ErrorAction SilentlyContinue |
        Measure-Object -Property Length -Sum).Sum
    if ($null -eq $sum) { return 0 }
    return [int64]$sum
}

function New-Candidate {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Reason
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return $null }
    $item = Get-Item -LiteralPath $Path -Force
    [PSCustomObject]@{
        Path = $item.FullName
        Name = $item.Name
        Reason = $Reason
        LastWriteTime = $item.LastWriteTime
        SizeGB = [math]::Round((Get-DirSizeBytes -Path $item.FullName) / 1GB, 3)
    }
}

$threshold = (Get-Date).AddDays(-1 * [Math]::Abs($DaysUnused))
$candidates = New-Object System.Collections.Generic.List[object]

# Known throwaway target dirs that should not be runtime sources.
$knownThrowaway = @(
    (Join-Path $Root "target-tests"),
    (Join-Path $Root "desktop\cortex-control-center\src-tauri\target-tests")
)
foreach ($path in $knownThrowaway) {
    $candidate = New-Candidate -Path $path -Reason "Known transient test target dir"
    if ($null -ne $candidate) { [void]$candidates.Add($candidate) }
}

# Daemon target-* dirs: keep hot runtime dirs unless stale.
$daemonRoot = Join-Path $Root "daemon-rs"
if (Test-Path -LiteralPath $daemonRoot -PathType Container) {
    $keepHot = @("target-control-center-dev", "target")
    $dirs = Get-ChildItem -LiteralPath $daemonRoot -Directory -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "target-*" }
    foreach ($dir in $dirs) {
        $isHot = $keepHot -contains $dir.Name
        if ($isHot -and $dir.LastWriteTime -ge $threshold) { continue }
        $reason = if ($isHot) {
            "Runtime target dir is stale (> $DaysUnused days)"
        } else {
            "Non-runtime isolated target dir"
        }
        $candidate = New-Candidate -Path $dir.FullName -Reason $reason
        if ($null -ne $candidate) { [void]$candidates.Add($candidate) }
    }
}

if ($IncludeBenchmarkArtifacts) {
    $benchmarkCandidates = @(
        (Join-Path $Root "benchmarking\tools\agent-memory-benchmark\.venv"),
        (Join-Path $Root "benchmarking\runs"),
        (Join-Path $Root "benchmarking\results")
    )
    foreach ($path in $benchmarkCandidates) {
        $candidate = New-Candidate -Path $path -Reason "Optional benchmark artifact cleanup"
        if ($null -ne $candidate) { [void]$candidates.Add($candidate) }
    }
}

if ($candidates.Count -eq 0) {
    Write-Host "[prune-build-bloat] no candidate directories found."
    exit 0
}

$sorted = $candidates | Sort-Object SizeGB -Descending
$totalGB = [math]::Round((($sorted | Measure-Object -Property SizeGB -Sum).Sum), 3)

Write-Host "[prune-build-bloat] candidates:"
$sorted | Format-Table Name, SizeGB, LastWriteTime, Reason, Path -AutoSize
Write-Host "[prune-build-bloat] total reclaimable (approx): $totalGB GB"

if (-not $Apply) {
    Write-Host "[prune-build-bloat] dry-run only. Re-run with -Apply to delete."
    exit 0
}

foreach ($entry in $sorted) {
    try {
        Remove-Item -LiteralPath $entry.Path -Recurse -Force -ErrorAction Stop
        Write-Host "[prune-build-bloat] removed $($entry.Path)"
    } catch {
        Write-Warning "[prune-build-bloat] failed to remove $($entry.Path): $($_.Exception.Message)"
    }
}

