$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $IsWindows -and $env:OS -ne 'Windows_NT') { throw 'GUI PTY check requires Windows.' }

$buildOutput = cargo build --quiet --message-format=json -p harness-platform --example gui_pty
if ($LASTEXITCODE -ne 0) { throw 'GUI PTY probe build failed.' }
$artifacts = @($buildOutput | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object {
    $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'gui_pty' -and
    $_.target.kind -contains 'example' -and $_.executable
})
if ($artifacts.Count -ne 1) { throw 'Cargo did not report exactly one GUI PTY executable.' }
$executable = (Resolve-Path -LiteralPath $artifacts[0].executable).Path

$previousProbe = $env:GUI_PTY_PROBE
try {
    $env:GUI_PTY_PROBE = $executable
    python -m unittest -q scripts.test_gui_pty_probe
    if ($LASTEXITCODE -ne 0) { throw 'GUI PTY probe failure checks failed.' }
} finally {
    $env:GUI_PTY_PROBE = $previousProbe
}

$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$runDirectory = Join-Path $tempRoot ('gui-pty-check-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $runDirectory | Out-Null
$report = Join-Path $runDirectory 'report.txt'
$process = $null
try {
    $process = Start-Process -FilePath $executable -ArgumentList @('--report', ('"{0}"' -f $report), '--work-dir', ('"{0}"' -f $runDirectory)) -PassThru -WindowStyle Hidden
    if (-not $process.WaitForExit(30000)) {
        Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
        $process.WaitForExit()
        throw 'GUI PTY probe exceeded 30 seconds.'
    }
    $details = if (Test-Path -LiteralPath $report) { Get-Content -LiteralPath $report -Raw }
    if ($details) { Write-Output $details }
    if ($process.ExitCode -ne 0) { throw "GUI PTY probe failed with exit code $($process.ExitCode)." }
    if (-not $details) { throw 'GUI PTY probe produced no report.' }
} finally {
    if ($null -ne $process -and -not $process.HasExited) { Stop-Process -Id $process.Id -ErrorAction SilentlyContinue }
    $resolvedDirectory = (Resolve-Path -LiteralPath $runDirectory).Path
    if (-not $resolvedDirectory.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
        -not (Split-Path -Leaf $resolvedDirectory).StartsWith('gui-pty-check-')) {
        throw 'Refusing to clean a directory outside the GUI PTY temporary directory.'
    }
    Remove-Item -LiteralPath $resolvedDirectory -Recurse -Force
}
