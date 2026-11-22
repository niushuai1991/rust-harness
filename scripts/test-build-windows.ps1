$ErrorActionPreference = 'Stop'
$sourceScript = Join-Path $PSScriptRoot 'build-windows.ps1'
$configuredVersion = [regex]::Match((Get-Content -LiteralPath $sourceScript -Raw), '(?m)^\$conptyVersion = ''([^'']+)''').Groups[1].Value
if (-not $configuredVersion) { throw 'Pinned ConPTY version not found.' }
$originalLocation = Get-Location
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$fixture = Join-Path $tempRoot ('rust-harness-build-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture | Out-Null
try {
    $scripts = Join-Path $fixture 'scripts'
    $assets = Join-Path $fixture 'src-tauri/binaries/conpty'
    $package = Join-Path $fixture 'package'
    New-Item -ItemType Directory -Force -Path $scripts, $assets, (Join-Path $package 'runtimes/win-x64/native'), (Join-Path $package 'build/native/runtimes/x64'), (Join-Path $fixture 'target/release/bundle/nsis') | Out-Null
    Copy-Item -LiteralPath $sourceScript -Destination (Join-Path $scripts 'build-windows.ps1')
    Set-Content -LiteralPath (Join-Path $package 'runtimes/win-x64/native/conpty.dll') -Value 'new dll'
    Set-Content -LiteralPath (Join-Path $package 'build/native/runtimes/x64/OpenConsole.exe') -Value 'new console'
    $archive = Join-Path $fixture 'fixture.zip'
    Compress-Archive -Path (Join-Path $package '*') -DestinationPath $archive
    Set-Content -LiteralPath (Join-Path $fixture 'src-tauri/tauri.conf.json') -Value '{"productName":"Test","version":"1.0"}'
    Set-Content -LiteralPath (Join-Path $fixture 'target/release/bundle/nsis/Test_1.0_x64-setup.exe') -Value 'installer'
    $global:conptyTestDownloadCount = 0
    $global:conptyTestNpmCalls = @()
    function Invoke-WebRequest { param($Uri, $OutFile, [switch]$UseBasicParsing) $global:conptyTestDownloadCount++; if (-not $UseBasicParsing) { throw 'Download must avoid interactive HTML parsing.' }; if (-not $Uri.EndsWith("/$configuredVersion")) { throw 'Unexpected ConPTY version.' }; Copy-Item -LiteralPath $archive -Destination $OutFile }
    function npm { $global:conptyTestNpmCalls += ($args -join ' '); $global:LASTEXITCODE = 0 }
    function python { $global:LASTEXITCODE = 0 }
    function Assert-True($condition, $message) { if (-not $condition) { throw $message } }
    $build = Join-Path $scripts 'build-windows.ps1'
    $versionFile = Join-Path $fixture 'src-tauri/binaries/conpty-version.txt'

    Set-Content -LiteralPath (Join-Path $assets 'conpty.dll') -Value 'stale dll'
    Set-Content -LiteralPath $versionFile -Value $configuredVersion
    & $build -SkipInstall | Out-Null
    Assert-True ($global:conptyTestDownloadCount -eq 1) 'Missing OpenConsole.exe must trigger a download.'
    Assert-True (Test-Path -LiteralPath (Join-Path $assets 'OpenConsole.exe')) 'Missing OpenConsole.exe after preparation.'
    Assert-True ((Get-Content -LiteralPath (Join-Path $assets 'conpty.dll') -Raw).Trim() -eq 'new dll') 'ConPTY DLL was not replaced.'

    & $build -SkipInstall | Out-Null
    Assert-True ($global:conptyTestDownloadCount -eq 1) 'A complete matching version must skip download.'

    $callsBefore = $global:conptyTestNpmCalls.Count
    & $build -PrepareOnly | Out-Null
    Assert-True ($global:conptyTestNpmCalls.Count -eq $callsBefore) 'Preparation-only must not run npm.'
    Assert-True ($global:conptyTestDownloadCount -eq 1) 'Preparation-only must reuse the matching cache.'

    Set-Content -LiteralPath $versionFile -Value 'older-version'
    & $build -SkipInstall | Out-Null
    Assert-True ($global:conptyTestDownloadCount -eq 2) 'A mismatched version must trigger a download.'
    Assert-True ((Get-Content -LiteralPath $versionFile -Raw).Trim() -eq $configuredVersion) 'Expected pinned version in cache metadata.'
    Assert-True (@($global:conptyTestNpmCalls | Where-Object { $_ -eq 'run tauri build' }).Count -eq 3) "Build must invoke Tauri directly without recursive npm entry; observed: $($global:conptyTestNpmCalls -join ' | ')."

    Remove-Item -LiteralPath (Join-Path $assets 'conpty.dll') -Force
    & $build -SkipInstall | Out-Null
    Assert-True ($global:conptyTestDownloadCount -eq 3) 'Missing conpty.dll must trigger a download.'

    Set-Content -LiteralPath $versionFile -Value '' -NoNewline
    & $build -SkipInstall | Out-Null
    Assert-True ($global:conptyTestDownloadCount -eq 4) 'An empty version file must trigger a download.'

    Set-Content -LiteralPath $versionFile -Value 'older-version'
    Remove-Item -LiteralPath (Join-Path $package 'build/native/runtimes/x64/OpenConsole.exe') -Force
    Compress-Archive -Path (Join-Path $package '*') -DestinationPath $archive -Force
    $failed = $false
    try { & $build -SkipInstall | Out-Null } catch { $failed = $true }
    Assert-True $failed 'An incomplete package must fail the build.'
    Assert-True ((Get-Content -LiteralPath $versionFile -Raw).Trim() -eq 'older-version') 'Incomplete package must not mark the cache current.'
    Write-Output 'PASS: ConPTY asset cache, missing file, and pinned version.'
} finally {
    Set-Location $originalLocation
    $resolved = (Resolve-Path -LiteralPath $fixture).Path
    if (-not $resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or -not (Split-Path -Leaf $resolved).StartsWith('rust-harness-build-test-')) { throw 'Invalid test fixture path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
