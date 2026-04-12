param(
    [string]$ToolsDir = (Join-Path $PSScriptRoot "tools")
)

$ErrorActionPreference = "Stop"

$repos = @(
    @{
        Name = "agent-memory-benchmark"
        Url = "https://github.com/vectorize-io/agent-memory-benchmark.git"
    },
    @{
        Name = "locomo"
        Url = "https://github.com/snap-research/locomo.git"
    }
)

New-Item -ItemType Directory -Path $ToolsDir -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $PSScriptRoot "runs") -Force | Out-Null

foreach ($repo in $repos) {
    $target = Join-Path $ToolsDir $repo.Name
    if (Test-Path $target) {
        Write-Host "Updating $($repo.Name)..."
        git -C $target pull --ff-only
    } else {
        Write-Host "Cloning $($repo.Name)..."
        git clone $repo.Url $target
    }
}

Write-Host ""
Write-Host "Benchmark tools ready in $ToolsDir"
