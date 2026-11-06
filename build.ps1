param([switch]$DebugBuild, [switch]$Check)
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    npm --prefix frontend ci
    if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
    npm --prefix frontend run build
    if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed' }
    if ($Check) {
        cargo fmt --check
        if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
        cargo clippy --all-targets --features custom-protocol -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
        cargo test --no-default-features --tests
        if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed' }
        node --test extension/rules.test.js
        if ($LASTEXITCODE -ne 0) { throw 'Extension tests failed' }
    }
    $cargoArgs = @('build', '--locked', '--features', 'custom-protocol,dev-tools')
    $profile = 'debug'
    if (-not $DebugBuild) { $cargoArgs += '--release'; $profile = 'release' }
    cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    New-Item -ItemType Directory -Force -Path build/bin | Out-Null
    Copy-Item -LiteralPath "target/$profile/dlm.exe" -Destination build/bin/dlm-rust.exe -Force
    Copy-Item -LiteralPath "target/$profile/dlm-testserver.exe" -Destination build/bin/dlm-testserver.exe -Force
    Copy-Item -LiteralPath "target/$profile/dlm-bench.exe" -Destination build/bin/dlm-bench.exe -Force
    Write-Host 'Ready: build/bin/dlm-rust.exe'
} finally { Pop-Location }
