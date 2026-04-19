param(
    [switch]$Apply
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Args
    )
    & git @Args
}

function Get-DirectorySizeGiB {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        return 0.0
    }
    $measure = Get-ChildItem -LiteralPath $Path -Recurse -Force -File -ErrorAction SilentlyContinue |
        Measure-Object -Property Length -Sum
    $sum = if ($null -ne $measure) { $measure.Sum } else { $null }
    if ($null -eq $sum -or $sum -le 0) {
        return 0.0
    }
    return [Math]::Round(($sum / 1GB), 3)
}

function Get-BloatCandidates {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    $paths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)

    $staticCandidates = @(
        "target-tests",
        "desktop/cortex-control-center/src-tauri/target-tests",
        ".tmp/pytest",
        "tmp/pytest-local"
    )
    foreach ($relative in $staticCandidates) {
        $absolute = Join-Path $RepoRoot $relative
        if (Test-Path -LiteralPath $absolute -PathType Container) {
            [void]$paths.Add($relative)
        }
    }

    $daemonRoot = Join-Path $RepoRoot "daemon-rs"
    if (Test-Path -LiteralPath $daemonRoot -PathType Container) {
        $daemonTargets = Get-ChildItem -LiteralPath $daemonRoot -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like "target-*" }
        foreach ($dir in $daemonTargets) {
            [void]$paths.Add(("daemon-rs/{0}" -f $dir.Name))
        }
    }

    return $paths
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repoRoot
try {
    $inside = (Invoke-Git -Args @("rev-parse", "--is-inside-work-tree") | Select-Object -First 1).Trim()
    if ($inside -ne "true") {
        throw "Not inside a git work tree: $repoRoot"
    }

    Write-Host "== Git Perf Health =="
    Write-Host "Repo: $repoRoot"

    $statusElapsed = (Measure-Command { Invoke-Git -Args @("status", "-sb") | Out-Null }).TotalMilliseconds
    $statusUnoElapsed = (Measure-Command { Invoke-Git -Args @("status", "-sb", "-uno") | Out-Null }).TotalMilliseconds
    $countObjects = Invoke-Git -Args @("count-objects", "-vH")
    $trackedCount = (Invoke-Git -Args @("ls-files") | Measure-Object).Count
    $statusScan = Invoke-Git -Args @("status", "--porcelain=v1", "-uall") 2>&1
    $statusScanLines = @($statusScan | Where-Object { $_ -is [string] })
    $untrackedCount = ($statusScanLines | Where-Object { $_ -like "?? *" } | Measure-Object).Count
    $scanWarnings = @($statusScanLines | Where-Object { $_ -like "warning: could not open directory*" })

    Write-Host ""
    Write-Host "Status timings:"
    Write-Host ("  git status -sb      : {0} ms" -f [Math]::Round($statusElapsed, 1))
    Write-Host ("  git status -sb -uno : {0} ms" -f [Math]::Round($statusUnoElapsed, 1))
    Write-Host ("  tracked files       : {0}" -f $trackedCount)
    Write-Host ("  untracked entries   : {0}" -f $untrackedCount)

    if ($scanWarnings.Count -gt 0) {
        Write-Host ""
        Write-Host "Scan warnings:"
        $scanWarnings | ForEach-Object { Write-Host "  $_" }
    }

    Write-Host ""
    Write-Host "Git object store:"
    $countObjects | ForEach-Object { Write-Host "  $_" }

    $bloatRows = @(foreach ($relative in (Get-BloatCandidates -RepoRoot $repoRoot)) {
        $absolute = Join-Path $repoRoot $relative
        if (Test-Path -LiteralPath $absolute -PathType Container) {
            [PSCustomObject]@{
                Path = $relative
                SizeGiB = Get-DirectorySizeGiB -Path $absolute
            }
        }
    })

    if ($bloatRows.Count -gt 0) {
        Write-Host ""
        Write-Host "Known bloat candidates:"
        $bloatRows | Sort-Object -Property SizeGiB -Descending | Format-Table -AutoSize | Out-String | Write-Host
    }

    if ($Apply) {
        Write-Host ""
        Write-Host "Applying local git performance knobs..."
        Invoke-Git -Args @("config", "--local", "feature.manyFiles", "true") | Out-Null
        Invoke-Git -Args @("config", "--local", "core.untrackedCache", "true") | Out-Null
        Invoke-Git -Args @("maintenance", "run", "--auto") | Out-Null
        Write-Host "Applied:"
        Write-Host "  git config --local feature.manyFiles true"
        Write-Host "  git config --local core.untrackedCache true"
        Write-Host "  git maintenance run --auto"
    } else {
        Write-Host ""
        Write-Host "Recommended next steps:"
        Write-Host "  1) npm run ops:prune-build-bloat"
        Write-Host "  2) npm run ops:prune-build-bloat:apply   (when dry-run looks safe)"
        Write-Host "  3) npm run ops:git-perf-apply"
    }
}
finally {
    Pop-Location
}
