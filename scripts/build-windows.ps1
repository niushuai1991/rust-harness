param([switch]$SkipInstall)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $IsWindows -and $env:OS -ne 'Windows_NT') { throw 'Windows build requires Windows.' }

# ConPTY 后备资产（Microsoft.Windows.Console.ConPTY，MIT）：conpty.dll + OpenConsole.exe
# 配对，供 GUI 宿主下绕开系统内置 conhost 的附着回归（详见 docs/specs/pty-execution-world.md）。
# 版本写死可追溯；dll 与 OpenConsole 必须同目录同版本配对（conpty.dll 按相对路径找 OpenConsole）。
$conptyVersion = '1.25.261109001'
$conptyDir = 'src-tauri/binaries/conpty'
if (-not (Test-Path (Join-Path $conptyDir 'conpty.dll'))) {
  $nupkg = Join-Path $env:TEMP "Microsoft.Windows.Console.ConPTY.$conptyVersion.nupkg"
  Invoke-WebRequest -Uri "https://www.nuget.org/api/v2/package/Microsoft.Windows.Console.ConPTY/$conptyVersion" -OutFile $nupkg
  $extract = Join-Path $env:TEMP "conpty-$conptyVersion"
  if (Test-Path $extract) { Remove-Item $extract -Recurse -Force }
  Expand-Archive -LiteralPath $nupkg -DestinationPath $extract -Force
  New-Item -ItemType Directory -Force -Path $conptyDir | Out-Null
  Copy-Item (Join-Path $extract 'runtimes/win-x64/native/conpty.dll') (Join-Path $conptyDir 'conpty.dll') -Force
  Copy-Item (Join-Path $extract 'build/native/runtimes/x64/OpenConsole.exe') (Join-Path $conptyDir 'OpenConsole.exe') -Force
  Remove-Item $extract -Recurse -Force
}

if (-not $SkipInstall) { npm ci --no-audit --no-fund; if ($LASTEXITCODE) { throw 'npm ci failed' } }
npm test; if ($LASTEXITCODE) { throw 'Frontend tests failed' }
npm run desktop:build
if ($LASTEXITCODE) { throw 'Tauri build failed' }
$config = Get-Content -LiteralPath src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
$installer = Join-Path 'target/release/bundle/nsis' ('{0}_{1}_x64-setup.exe' -f $config.productName, $config.version)
python scripts/verify-installer.py $installer --application target/release/rust-harness-desktop.exe
if ($LASTEXITCODE) { throw 'Installer must request administrator privileges for protected installation paths.' }
Get-Item -LiteralPath $installer
