# Prepare an isolated Windows Sandbox. Nothing is installed on the host.
[CmdletBinding()]
param([switch]$Launch, [switch]$DesktopRunner)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cache = Join-Path $repo '.cache'
# Reject junctions/symlinks along every writable host path.
for ($path = $cache; $path; $path = Split-Path -Path $path -Parent) {
    if ((Test-Path -LiteralPath $path) -and ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Reparse point is not allowed: $path"
    }
}
$configPath = Join-Path $repo 'src-tauri\tauri.conf.json'
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
$currentVersion = [string]$config.version
if ($currentVersion -notmatch '^\d+\.\d+\.\d+$') { throw 'Expected a numeric stable Tauri version.' }
$feature = Get-CimInstance Win32_OptionalFeature -Filter "Name = 'Containers-DisposableClientVM'"
$computer = Get-CimInstance Win32_ComputerSystem
$sandbox = Join-Path $env:WINDIR 'System32\WindowsSandbox.exe'
if (-not (Test-Path -LiteralPath $sandbox) -or $feature.InstallState -ne 1 -or -not $computer.HypervisorPresent) {
    throw 'Windows Sandbox is unavailable; no host feature or boot configuration will be changed.'
}
$old = Join-Path $repo 'target\release\bundle\nsis\Rust Harness_0.1.2_x64-standard-setup.exe'
$new = Join-Path $repo ("target\x86_64-pc-windows-msvc\release\bundle\nsis\Rust Harness_" + $currentVersion + '_x64-setup.exe')
$runtimes = @(Get-ChildItem -LiteralPath (Join-Path $repo 'target\.tauri\x64') -Recurse -Filter 'MicrosoftEdgeWebView2RuntimeInstallerX64.exe' -File)
if ($runtimes.Count -ne 1) { throw 'Expected exactly one cached offline WebView2 installer.' }
$sources = [ordered]@{
    'installer-0.1.2.exe' = $old
    'installer-current.exe' = $new
    'webview2-offline.exe' = $runtimes[0].FullName
    'p1-sandbox-guest.ps1' = (Join-Path $PSScriptRoot 'p1-sandbox-guest.ps1')
}
if ($DesktopRunner) {
    $sources['node.exe'] = (Get-Command node.exe -CommandType Application).Source
    $sources['p1-desktop-cdp.mjs'] = Join-Path $PSScriptRoot 'p1-desktop-cdp.mjs'
}
foreach ($source in $sources.Values) {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing asset: $source" }
}
$runId = [guid]::NewGuid().ToString('N')
$run = Join-Path $cache ("p1-sandbox-" + $runId)
if (-not $run.StartsWith($cache + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Run path escaped repository cache.' }
$assets = Join-Path $run 'assets'
$results = Join-Path $run 'results'
New-Item -ItemType Directory -Path $assets,$results -Force | Out-Null
$assetInfo = @()
foreach ($entry in $sources.GetEnumerator()) {
    $destination = Join-Path $assets $entry.Key
    Copy-Item -LiteralPath $entry.Value -Destination $destination
    $assetInfo += [ordered]@{ name=$entry.Key; sha256=(Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash; bytes=(Get-Item -LiteralPath $destination).Length }
}
$manifest = [ordered]@{
    runId=$runId; hostComputerName=$env:COMPUTERNAME; desktopRunner=[bool]$DesktopRunner; currentVersion=$currentVersion
    preparedAt=[DateTime]::UtcNow.ToString('o'); assets=$assetInfo
    host=[ordered]@{ os=[Environment]::OSVersion.VersionString; sandboxFeature=$feature.InstallState; hypervisor=$computer.HypervisorPresent; memoryBytes=$computer.TotalPhysicalMemory }
    isolation=[ordered]@{networking='Disable';memoryMB=4096;assetsReadOnly=$true;writableHostPath=$results}
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $assets 'manifest.json') -Encoding UTF8
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $results 'host-environment.json') -Encoding UTF8
$assetXml = [Security.SecurityElement]::Escape($assets)
$resultsXml = [Security.SecurityElement]::Escape($results)
$configuration = @"
<Configuration>
  <vGPU>Disable</vGPU>
  <Networking>Disable</Networking>
  <AudioInput>Disable</AudioInput>
  <VideoInput>Disable</VideoInput>
  <PrinterRedirection>Disable</PrinterRedirection>
  <ClipboardRedirection>Disable</ClipboardRedirection>
  <MemoryInMB>4096</MemoryInMB>
  <MappedFolders>
    <MappedFolder><HostFolder>$assetXml</HostFolder><SandboxFolder>C:\P1Assets</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$resultsXml</HostFolder><SandboxFolder>C:\P1Results</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>
  </MappedFolders>
  <LogonCommand><Command>powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\P1Assets\p1-sandbox-guest.ps1</Command></LogonCommand>
</Configuration>
"@
$wsb = Join-Path $run 'acceptance.wsb'
$configuration | Set-Content -LiteralPath $wsb -Encoding UTF8
[ordered]@{status='prepared';runId=$runId;configuration=$wsb;results=$results;launched=[bool]$Launch} | ConvertTo-Json
if ($Launch) {
    $existingSandbox = @(Get-Process -Name WindowsSandboxRemoteSession -ErrorAction SilentlyContinue)
    if ($existingSandbox.Count -gt 0) {
        throw "Windows Sandbox is already in use (PID: $($existingSandbox.Id -join ', ')); close that disposable session before launching this isolated acceptance run."
    }
    Start-Process -FilePath $sandbox -ArgumentList ('"' + $wsb + '"') -WindowStyle Hidden | Out-Null
}
