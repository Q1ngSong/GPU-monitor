param(
    [switch]$DesktopShortcut,
    [switch]$Open
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseDirectory = Join-Path $projectRoot 'src-tauri\target\release'
$appDirectory = Join-Path $projectRoot 'app'
$appExecutable = Join-Path $appDirectory 'gpu-monitor.exe'
$cliExecutable = Join-Path $appDirectory 'gpu-monitor-cli.exe'

foreach ($file in @('gpu-monitor.exe', 'gpu-monitor-cli.exe')) {
    if (-not (Test-Path -LiteralPath (Join-Path $releaseDirectory $file) -PathType Leaf)) {
        throw "Missing release file: $file. Build the desktop app first."
    }
}

# Only replace this project's portable copy; user settings live in AppData.
$running = Get-Process -Name 'gpu-monitor' -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $appExecutable }
foreach ($process in $running) {
    Stop-Process -Id $process.Id
    if (-not $process.WaitForExit(10000)) {
        throw 'GPU Monitor is still closing. Please try the update again.'
    }
}
New-Item -ItemType Directory -Path $appDirectory -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $releaseDirectory 'gpu-monitor.exe') -Destination $appExecutable -Force
Copy-Item -LiteralPath (Join-Path $releaseDirectory 'gpu-monitor-cli.exe') -Destination $cliExecutable -Force

if ($DesktopShortcut) {
    $shortcutPath = Join-Path ([Environment]::GetFolderPath('Desktop')) 'GPU Monitor.lnk'
    $shortcutShell = New-Object -ComObject WScript.Shell
    $shortcut = $shortcutShell.CreateShortcut($shortcutPath)
    if ((Test-Path -LiteralPath $shortcutPath) -and $shortcut.TargetPath -ne $appExecutable) {
        throw 'A different GPU Monitor shortcut already exists. It has not been replaced.'
    }
    $shortcut.TargetPath = $appExecutable
    $shortcut.WorkingDirectory = $appDirectory
    $shortcut.IconLocation = "$appExecutable,0"
    $shortcut.Description = 'GPU and CPU monitoring'
    $shortcut.Save()
    Write-Output "Desktop shortcut: $shortcutPath"
}

Write-Output "App: $appExecutable"
Write-Output "CLI: $cliExecutable"
if ($Open) { Start-Process -FilePath $appExecutable | Out-Null }
