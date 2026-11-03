param([switch]$Offline, [switch]$SkipInstall)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $IsWindows -and $env:OS -ne 'Windows_NT') { throw 'Windows build requires Windows.' }
if (-not $SkipInstall) { npm ci --no-audit --no-fund; if ($LASTEXITCODE) { throw 'npm ci failed' } }
npm test; if ($LASTEXITCODE) { throw 'Frontend tests failed' }
if ($Offline) { npm run desktop:offline } else { npm run desktop:build }
if ($LASTEXITCODE) { throw 'Tauri build failed' }
Get-ChildItem -Path target/release/bundle/nsis -Filter '*.exe'
