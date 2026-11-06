param([switch]$DebugBuild, [switch]$Check, [switch]$Installer)
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
    if ($Installer) {
        # Compiled panic messages carry source paths, including the user folder.
        $env:CARGO_ENCODED_RUSTFLAGS = "--remap-path-prefix=$HOME=~"
        npx --yes @tauri-apps/cli@2 build --bundles nsis
        if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
        Write-Host 'Ready: target/release/bundle/nsis'
        return
    }
    $cargoArgs = @('build', '--locked', '--features', 'custom-protocol,dev-tools')
    $profile = 'debug'
    if (-not $DebugBuild) { $cargoArgs += '--release'; $profile = 'release' }
    cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    New-Item -ItemType Directory -Force -Path build/bin | Out-Null
    Copy-Item -LiteralPath "target/$profile/acqua.exe" -Destination build/bin/acqua.exe -Force
    Copy-Item -LiteralPath "target/$profile/dlm-testserver.exe" -Destination build/bin/dlm-testserver.exe -Force
    Copy-Item -LiteralPath "target/$profile/dlm-bench.exe" -Destination build/bin/dlm-bench.exe -Force
    Write-Host 'Ready: build/bin/acqua.exe'
} finally {
    Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
    Pop-Location
}
