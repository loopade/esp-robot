param(
    [string]$ContainerName = "arm-pc-ros",
    [switch]$StopXbox
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

if ($StopXbox) {
    Write-Host "Stopping Xbox controller scripts..."
    Get-CimInstance Win32_Process |
        Where-Object {
            $_.CommandLine -like '*xbox_control.ps1*' -and
            $_.CommandLine -notlike '*Get-CimInstance Win32_Process*'
        } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
}

Write-Host "Stopping PC ROS container..."
podman rm -f $ContainerName 2>$null | Out-Null

Write-Host "Container stopped."
