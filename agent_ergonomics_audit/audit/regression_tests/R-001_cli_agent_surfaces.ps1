$ErrorActionPreference = "Stop"

$repo = Resolve-Path "$PSScriptRoot\..\..\..\daemon-rs"
Push-Location $repo
try {
    $help = (cargo run --quiet -- --help) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw "help exited $LASTEXITCODE" }
    if ($help -notmatch "cortex capabilities --json") { throw "help missing capabilities entrypoint" }
    if ($help -notmatch "cortex robot-docs guide") { throw "help missing robot-docs entrypoint" }

    $capabilities = cargo run --quiet -- capabilities --json | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "capabilities exited $LASTEXITCODE" }
    if ($capabilities.contract_version -ne "1") { throw "unexpected capabilities contract version" }
    if ($capabilities.commands.paths.output -ne "json") { throw "paths command output contract missing" }

    $guide = (cargo run --quiet -- robot-docs guide) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw "robot-docs exited $LASTEXITCODE" }
    if ($guide -notmatch "Danger gates:") { throw "robot guide missing danger gates" }

    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $unknownLines = & cargo run --quiet -- capability 2>&1
    $unknownExit = $LASTEXITCODE
    $ErrorActionPreference = $previousErrorAction
    $unknown = $unknownLines -join "`n"
    if ($unknownExit -ne 1) { throw "unknown command should exit 1" }
    if ($unknown -notmatch "cortex capabilities --json") { throw "unknown command missing suggestion" }
}
finally {
    Pop-Location
}
