param([switch]$SkipInstall)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $IsWindows -and $env:OS -ne 'Windows_NT') { throw 'Windows build requires Windows.' }
if (-not $SkipInstall) { npm ci --no-audit --no-fund; if ($LASTEXITCODE) { throw 'npm ci failed' } }
npm test; if ($LASTEXITCODE) { throw 'Frontend tests failed' }
npm run desktop:build
if ($LASTEXITCODE) { throw 'Tauri build failed' }
$config = Get-Content -LiteralPath src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
$installer = Join-Path 'target/release/bundle/nsis' ('{0}_{1}_x64-setup.exe' -f $config.productName, $config.version)
python scripts/verify-installer.py $installer --application target/release/rust-harness-desktop.exe
if ($LASTEXITCODE) { throw 'Installer must request administrator privileges for protected installation paths.' }
Get-Item -LiteralPath $installer
