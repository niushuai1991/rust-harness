param([string]$Executable = "$PSScriptRoot/../target/release/rust-harness-desktop.exe")
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $Executable).Path
$process = Start-Process -FilePath $exe -PassThru -WindowStyle Hidden
try {
    Start-Sleep -Seconds 5
    if ($process.HasExited) { throw "Desktop exited early: $($process.ExitCode)" }
    $second = Start-Process -FilePath $exe -PassThru -WindowStyle Hidden
    if (-not $second.WaitForExit(15000)) { Stop-Process -Id $second.Id; throw 'Single-instance guard did not close the second instance.' }
    if ($process.HasExited) { throw 'Original desktop process unexpectedly exited.' }
    Write-Output 'PASS: desktop launch and single-instance activation. No model request or source-project write was made.'
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id }
}
