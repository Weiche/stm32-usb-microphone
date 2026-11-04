$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    cargo fmt --all -- --check
    if ($LASTEXITCODE) { throw 'rustfmt failed' }
    cargo build --locked --release
    if ($LASTEXITCODE) { throw 'firmware build failed' }
    cargo clippy --locked --release -- -D warnings
    if ($LASTEXITCODE) { throw 'firmware clippy failed' }
    cargo test --locked --lib --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE) { throw 'host tests failed' }
    cargo clippy --locked --lib --tests --target x86_64-pc-windows-msvc -- -D warnings
    if ($LASTEXITCODE) { throw 'host clippy failed' }
} finally {
    Pop-Location
}
