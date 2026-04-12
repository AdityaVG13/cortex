param(
    [string]$ToolsDir = (Join-Path $PSScriptRoot "tools")
)

$ErrorActionPreference = "Stop"

$lockPath = Join-Path $PSScriptRoot "benchmarks.lock.json"

if (-not (Test-Path $lockPath)) {
    throw "Missing lock file: $lockPath"
}

$lock = Get-Content $lockPath -Raw | ConvertFrom-Json

New-Item -ItemType Directory -Path $ToolsDir -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $PSScriptRoot "runs") -Force | Out-Null

foreach ($repo in $lock.tools) {
    $target = Join-Path $ToolsDir $repo.name
    if (Test-Path $target) {
        Write-Host "Updating $($repo.name)..."
        git -C $target fetch --all --tags --prune | Out-Host
    } else {
        Write-Host "Cloning $($repo.name)..."
        git clone $repo.url $target | Out-Host
    }

    git -C $target checkout $repo.commit | Out-Host
}

Write-Host ""
Write-Host "Benchmark tools ready in $ToolsDir"
Write-Host "Pinned tool versions are defined in $lockPath"
