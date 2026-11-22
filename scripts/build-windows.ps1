param([switch]$SkipInstall)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Windows build requires Windows.' }

# ConPTY 后备资产（Microsoft.Windows.Console.ConPTY，MIT）：conpty.dll + OpenConsole.exe
# 配对，供 GUI 宿主下绕开系统内置 conhost 的附着回归（详见 docs/specs/pty-execution-world.md）。
# 版本写死可追溯；dll 与 OpenConsole 必须同目录同版本配对（conpty.dll 按相对路径找 OpenConsole）。
$conptyVersion = '1.25.261109001'
$conptyDir = 'src-tauri/binaries/conpty'
$versionFile = 'src-tauri/binaries/conpty-version.txt'
$cachedVersion = if (Test-Path -LiteralPath $versionFile -PathType Leaf) {
  $versionContents = Get-Content -LiteralPath $versionFile -Raw
  if ($null -ne $versionContents) { $versionContents.Trim() }
}
if ($cachedVersion -eq $conptyVersion -and
    (Test-Path -LiteralPath (Join-Path $conptyDir 'conpty.dll') -PathType Leaf) -and
    (Test-Path -LiteralPath (Join-Path $conptyDir 'OpenConsole.exe') -PathType Leaf)) {
  Write-Output "Using cached ConPTY $conptyVersion."
} else {
  $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
  $tempDir = Join-Path $tempRoot ('rust-harness-conpty-' + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $tempDir | Out-Null
  try {
    # NuGet 包内容为 ZIP；直接使用 .zip 后缀，兼容 Expand-Archive。
    $zip = Join-Path $tempDir 'conpty.zip'
    $extract = Join-Path $tempDir 'extracted'
    Invoke-WebRequest -UseBasicParsing -Uri "https://www.nuget.org/api/v2/package/Microsoft.Windows.Console.ConPTY/$conptyVersion" -OutFile $zip
    Expand-Archive -LiteralPath $zip -DestinationPath $extract -Force
    $dll = Join-Path $extract 'runtimes/win-x64/native/conpty.dll'
    $console = Join-Path $extract 'build/native/runtimes/x64/OpenConsole.exe'
    if (-not (Test-Path -LiteralPath $dll -PathType Leaf) -or -not (Test-Path -LiteralPath $console -PathType Leaf)) { throw 'ConPTY package is missing required files.' }
    New-Item -ItemType Directory -Force -Path $conptyDir | Out-Null
    if (Test-Path -LiteralPath $versionFile) { Remove-Item -LiteralPath $versionFile -Force }
    Copy-Item -LiteralPath $dll -Destination (Join-Path $conptyDir 'conpty.dll') -Force
    Copy-Item -LiteralPath $console -Destination (Join-Path $conptyDir 'OpenConsole.exe') -Force
    Set-Content -LiteralPath $versionFile -Value $conptyVersion -Encoding UTF8
  } finally {
    $resolved = (Resolve-Path -LiteralPath $tempDir).Path
    if (-not $resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or -not (Split-Path -Leaf $resolved).StartsWith('rust-harness-conpty-')) { throw 'Invalid ConPTY temporary directory.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
  }
}

if (-not $SkipInstall) { npm ci --no-audit --no-fund; if ($LASTEXITCODE) { throw 'npm ci failed' } }
npm test; if ($LASTEXITCODE) { throw 'Frontend tests failed' }
npm run tauri -- build
if ($LASTEXITCODE) { throw 'Tauri build failed' }
$config = Get-Content -LiteralPath src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
$installer = Join-Path 'target/release/bundle/nsis' ('{0}_{1}_x64-setup.exe' -f $config.productName, $config.version)
python scripts/verify-installer.py $installer --application target/release/rust-harness-desktop.exe
if ($LASTEXITCODE) { throw 'Installer must request administrator privileges for protected installation paths.' }
Get-Item -LiteralPath $installer
