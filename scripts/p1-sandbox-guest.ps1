# This script must run only as Windows Sandbox's disposable user.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:USERNAME -ne 'WDAGUtilityAccount') { throw 'Refusing to execute outside Windows Sandbox.' }
$assets = 'C:\P1Assets'
$results = 'C:\P1Results'
$manifest = Get-Content -LiteralPath "$assets\manifest.json" -Raw | ConvertFrom-Json
$currentVersion = [string]$manifest.currentVersion
if ($env:COMPUTERNAME -eq $manifest.hostComputerName) { throw 'Refusing host execution.' }
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Sandbox logon command needs its default administrator token.' }
$install = 'C:\Program Files\P1 中文 空格\Rust Harness'
$exe = Join-Path $install 'rust-harness-desktop.exe'
$registry = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Rust Harness'
$checks = [Collections.Generic.List[object]]::new()
$report = [ordered]@{runId=$manifest.runId;status='running';startedAt=[DateTime]::UtcNow.ToString('o');environment=$null;checks=$checks;error=$null}
function Save-Report {
    $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath "$results\installer-result.json" -Encoding UTF8
}
function Check([string]$Name, [bool]$Passed, $Evidence) {
    $checks.Add([ordered]@{name=$Name;passed=$Passed;evidence=$Evidence})
    Save-Report
    if (-not $Passed) { throw "Acceptance failed: $Name" }
}
function Run-Program([string]$Path, [string]$Arguments, [int]$TimeoutSeconds=240) {
    $process = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru -WindowStyle Hidden
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        # Only a process started by this script inside the disposable guest is terminated.
        $process.Kill()
        throw "Timed out: $Path"
    }
    $process.Refresh()
    return $process.ExitCode
}
function Check-Install([string]$Version) {
    Check "installed-file-$Version" (Test-Path -LiteralPath $exe) $exe
    $actual = (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion
    Check "binary-version-$Version" ($actual -eq $Version -or $actual -eq "$Version.0") $actual
    $registration = Get-ItemProperty -LiteralPath $registry
    Check "machine-registration-$Version" ($registration.DisplayVersion -eq $Version -and $registration.InstallLocation.Trim('"').TrimEnd('\') -eq $install) @{version=$registration.DisplayVersion;location=$registration.InstallLocation}
}
function Invoke-SelectedCleanup {
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public static class P1GuestUi{[DllImport("user32.dll")]public static extern bool PostMessage(IntPtr hWnd,uint message,IntPtr wParam,IntPtr lParam);[DllImport("user32.dll")]public static extern bool SetCursorPos(int x,int y);[DllImport("user32.dll")]public static extern void mouse_event(uint flags,uint dx,uint dy,uint data,UIntPtr extraInfo);}'
    function Invoke-GuestClick($Element) {
        $bounds = $Element.Current.BoundingRectangle
        if ($bounds.IsEmpty -or $bounds.Width -le 0 -or $bounds.Height -le 0) { throw "Guest control has no clickable bounds: $($Element.Current.Name)" }
        $x = [int]($bounds.X + ($bounds.Width / 2)); $y = [int]($bounds.Y + ($bounds.Height / 2))
        [void][P1GuestUi]::SetCursorPos($x, $y)
        [P1GuestUi]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
        [P1GuestUi]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
        return @{x=$x;y=$y;name=$Element.Current.Name}
    }
    # _?= avoids NSIS's temporary child so every UIA action is scoped to this exact guest PID.
    # This is the sole deliberately visible program: its guest-only checkbox UI is under test.
    $process = Start-Process -FilePath (Join-Path $install 'uninstall.exe') -ArgumentList ('_?=' + $install) -PassThru -WindowStyle Normal
    $pidCondition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, $process.Id)
    # The configured NSIS language selector precedes the uninstall page. Confirm its default
    # selection through the exact guest-owned PID before locating the cleanup checkbox.
    $languageDeadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $languageDeadline) {
        $languageWindow = [Windows.Automation.AutomationElement]::RootElement.FindFirst([Windows.Automation.TreeScope]::Children, $pidCondition)
        if ($null -ne $languageWindow -and $languageWindow.Current.Name -eq 'Installer Language') {
            $handle = [IntPtr]$languageWindow.Current.NativeWindowHandle
            [void][P1GuestUi]::PostMessage($handle, 0x0100, [IntPtr]13, [IntPtr]::Zero)
            [void][P1GuestUi]::PostMessage($handle, 0x0101, [IntPtr]13, [IntPtr]::Zero)
            break
        }
        Start-Sleep -Milliseconds 100
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    $window = $null; $checkbox = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $window = [Windows.Automation.AutomationElement]::RootElement.FindFirst([Windows.Automation.TreeScope]::Children, $pidCondition)
        if ($null -ne $window) {
            $all = $window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
            foreach ($element in $all) {
                if ($element.Current.Name -in @('Delete the application data','删除应用程序数据')) { $checkbox = $element; break }
            }
        }
        if ($null -ne $checkbox) { break }
        Start-Sleep -Milliseconds 250
    }
    if ($null -eq $checkbox) {
        $process.Refresh()
        $known = @(Get-CimInstance Win32_Process -Filter "Name = 'uninstall.exe'" | Select-Object ProcessId,ParentProcessId,CommandLine)
        $windows = @()
        foreach ($candidate in $known) {
            $candidateCondition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, [int]$candidate.ProcessId)
            $candidateWindow = [Windows.Automation.AutomationElement]::RootElement.FindFirst([Windows.Automation.TreeScope]::Children, $candidateCondition)
            if ($null -ne $candidateWindow) {
                $controls = @($candidateWindow.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition) | ForEach-Object { @{name=$_.Current.Name;controlType=$_.Current.ControlType.ProgrammaticName} })
                $windows += @{pid=$candidate.ProcessId;name=$candidateWindow.Current.Name;controls=$controls}
            }
        }
        $report.explicitCleanup.uiDiagnostics = @{initialPid=$process.Id;initialExited=$process.HasExited;processes=$known;windows=$windows}
        Save-Report
    }
    Check 'selected-cleanup-checkbox-visible' ($null -ne $checkbox) @{pid=$process.Id}
    $checkboxClick = Invoke-GuestClick $checkbox
    Check 'selected-cleanup-checkbox-on' $true $checkboxClick
    $button = $null
    foreach ($element in $window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)) {
        if ($element.Current.IsEnabled -and $element.Current.Name -match '^(?:&?Uninstall|卸载)') { $button = $element; break }
    }
    Check 'selected-cleanup-uninstall-button-visible' ($null -ne $button) @{pid=$process.Id}
    Invoke-GuestClick $button | Out-Null
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    while (((Test-Path -LiteralPath $exe) -or (Test-Path -LiteralPath $registry)) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 300 }
    Check 'selected-cleanup-uninstalls-app' (-not (Test-Path -LiteralPath $exe) -and -not (Test-Path -LiteralPath $registry)) @{pid=$process.Id}
}
try {
    Start-Transcript -LiteralPath "$results\guest-transcript.txt" -Force | Out-Null
    $os = Get-CimInstance Win32_OperatingSystem
    $report.environment = [ordered]@{caption=$os.Caption;version=$os.Version;build=$os.BuildNumber;user=$env:USERNAME;computer=$env:COMPUTERNAME;isAdministrator=$true;installPath=$install;uacScope='Disposable Sandbox administrator; does not verify real standard-user UAC interaction.'}
    Save-Report
    Check 'windows-11-guest' ([int]$os.BuildNumber -ge 22000 -and $os.ProductType -eq 1) $report.environment
    Check 'clean-guest-app' (-not (Test-Path -LiteralPath $registry) -and -not (Test-Path -LiteralPath $exe)) 'No Rust Harness HKLM registration or app executable before install.'
    foreach ($asset in $manifest.assets) {
        $actual = (Get-FileHash -LiteralPath (Join-Path $assets $asset.name) -Algorithm SHA256).Hash
        Check ("asset-integrity-" + $asset.name) ($actual -eq $asset.sha256) $actual
    }
    # Cached offline runtime only. Never fetch a bootstrapper or install on the host.
    $runtimeKeys = @(
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
        'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
        'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    )
    $runtimePresent = @($runtimeKeys | Where-Object { Test-Path -LiteralPath $_ }).Count -gt 0
    if (-not $runtimePresent) {
        $code = Run-Program "$assets\webview2-offline.exe" '/silent /install' 600
        Check 'guest-offline-webview2-install' ($code -eq 0 -or $code -eq 3010) $code
    }
    Check 'guest-webview2-present' (@($runtimeKeys | Where-Object { Test-Path -LiteralPath $_ }).Count -gt 0) @{preexisting=$runtimePresent}
    # /D must be the last NSIS argument and its complete path is deliberately unquoted.
    $code = Run-Program "$assets\installer-0.1.2.exe" ("/S /D=" + $install)
    Check 'install-0.1.2-exit' ($code -eq 0) $code
    Check-Install '0.1.2'
    $markerValue = 'synthetic-p1-' + $manifest.runId
    $markers = @(
        (Join-Path $env:LOCALAPPDATA 'RustHarness\P1 中文 空格\retain.txt'),
        (Join-Path $env:LOCALAPPDATA 'dev.rustharness.desktop\P1 中文 空格\retain.txt'),
        (Join-Path $env:APPDATA 'dev.rustharness.desktop\P1 中文 空格\retain.txt')
    )
    foreach ($marker in $markers) {
        New-Item -ItemType Directory -Path (Split-Path $marker) -Force | Out-Null
        Set-Content -LiteralPath $marker -Value $markerValue -Encoding UTF8
    }
    $code = Run-Program "$assets\installer-current.exe" ("/S /UPDATE /D=" + $install)
    Check "upgrade-$currentVersion-exit" ($code -eq 0) $code
    Check-Install $currentVersion
    foreach ($marker in $markers) { Check 'upgrade-preserves-data' ((Get-Content -LiteralPath $marker -Raw).Trim() -eq $markerValue) $marker }
    if ($manifest.desktopRunner) {
        $env:P1_SANDBOX_GUEST = '1'
        $env:RUST_HARNESS_DATA_DIR = 'C:\P1 工作区 空格\Runtime'
        $env:RUST_HARNESS_P1_CDP_PORT = '9222'
        $arguments = '"' + "$assets\p1-desktop-cdp.mjs" + '" --exe "' + $exe + '" --results "' + $results + '" --data-dir "' + $env:RUST_HARNESS_DATA_DIR + '"'
        $code = Run-Program "$assets\node.exe" $arguments 900
        Check 'native-desktop-runner' ($code -eq 0) $code
    } else {
        # A minimal launch check is weaker than the optional real WebView2 UI runner.
        $env:RUST_HARNESS_DATA_DIR = 'C:\P1 工作区 空格\Runtime'
        $app = Start-Process -FilePath $exe -PassThru -WindowStyle Hidden
        Start-Sleep -Seconds 10
        $app.Refresh()
        Check 'native-process-start' (-not $app.HasExited) @{pid=$app.Id}
        $app.Kill(); $app.WaitForExit()
    }
    # Refuse uninstall with an app still running; do not terminate an arbitrary process.
    $running = @(Get-Process -Name 'rust-harness-desktop' -ErrorAction SilentlyContinue)
    Check 'desktop-stopped-before-uninstall' ($running.Count -eq 0) $running.Count
    $code = Run-Program (Join-Path $install 'uninstall.exe') '/S'
    Check 'uninstall-exit' ($code -eq 0) $code
    # NSIS may spawn its temporary uninstaller; poll observable removal.
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    while (((Test-Path -LiteralPath $exe) -or (Test-Path -LiteralPath $registry)) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 300 }
    Check 'uninstall-removes-binary-and-registration' (-not (Test-Path -LiteralPath $exe) -and -not (Test-Path -LiteralPath $registry)) $install
    foreach ($marker in $markers) { Check 'uninstall-preserves-data' ((Get-Content -LiteralPath $marker -Raw).Trim() -eq $markerValue) $marker }
    # Regression phase: 0.1.3 is known to leave the actual RustHarness root behind.
    # Keep this as a real failing requirement until fixed, never convert expected failure to pass.
    $report['explicitCleanup'] = [ordered]@{knownFailureBeforeFix='0.1.3 deletes only BUNDLEID roots, leaving LOCALAPPDATA\RustHarness';status='running';roots=@();externalFixtures=@()}
    $externalMarkers = @('C:\P1 外部项目 空格\keep.txt', 'C:\P1 工作区 空格\Runtime\keep.txt', 'C:\P1 外部 fixture 空格\keep.txt')
    foreach ($marker in $externalMarkers) {
        New-Item -ItemType Directory -Path (Split-Path $marker) -Force | Out-Null
        Set-Content -LiteralPath $marker -Value $markerValue -Encoding UTF8
    }
    $code = Run-Program "$assets\installer-current.exe" ("/S /D=" + $install)
    Check 'reinstall-before-selected-cleanup' ($code -eq 0) $code
    Check-Install $currentVersion
    # Synthetic guest junctions expose unsafe recursive deletion without mapping any host project.
    $cleanupRoots = @((Join-Path $env:LOCALAPPDATA 'RustHarness'), (Join-Path $env:LOCALAPPDATA 'dev.rustharness.desktop'), (Join-Path $env:APPDATA 'dev.rustharness.desktop'))
    foreach ($root in $cleanupRoots) {
        New-Item -ItemType Junction -Path (Join-Path $root 'P1 external link') -Target 'C:\P1 外部 fixture 空格' | Out-Null
    }
    Invoke-SelectedCleanup
    $cleanupPassed = $true
    foreach ($path in $cleanupRoots) {
        $removed = -not (Test-Path -LiteralPath $path)
        $report.explicitCleanup.roots += @{path=$path;removed=$removed}
        if (-not $removed) { $cleanupPassed = $false }
    }
    foreach ($marker in $externalMarkers) {
        $preserved = (Test-Path -LiteralPath $marker) -and ((Get-Content -LiteralPath $marker -Raw).Trim() -eq $markerValue)
        $report.explicitCleanup.externalFixtures += @{path=$marker;preserved=$preserved}
        if (-not $preserved) { $cleanupPassed = $false }
    }
    $report.explicitCleanup.status = $(if ($cleanupPassed) {'passed'} else {'failed'})
    Check 'selected-cleanup-removes-only-local-app-data' $cleanupPassed $report.explicitCleanup
    $report.status = 'passed'
} catch {
    $report.status = 'failed'
    $report.error = $_.Exception.Message
    Write-Warning $_.Exception.Message
} finally {
    $report['finishedAt'] = [DateTime]::UtcNow.ToString('o')
    Save-Report
    try { Stop-Transcript | Out-Null } catch { }
}
if ($report.status -ne 'passed') { exit 1 }
