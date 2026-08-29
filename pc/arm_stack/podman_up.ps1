param(
    [string]$Image = "localhost/arm-pc-ros:jazzy",
    [string]$ContainerName = "arm-pc-ros",
    [int]$RosDomainId = 30,
    [switch]$Launch
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

$workspace = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$containerWorkspace = "/workspace/arm-stack"
$pcIp = (
    Get-NetIPAddress -AddressFamily IPv4 |
    Where-Object {
        $_.IPAddress -notlike "169.254.*" -and
        $_.IPAddress -ne "127.0.0.1" -and
        $_.PrefixOrigin -ne "WellKnown"
    } |
    Select-Object -First 1 -ExpandProperty IPAddress
)

function Start-PodmanMachineSafe {
    $machineList = podman machine list --format "{{.Name}} {{.Running}}" 2>$null
    if ($LASTEXITCODE -eq 0) {
        foreach ($line in $machineList) {
            if ($line -match "^\S+\s+true$") {
                return
            }
        }
    }

    podman machine start | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to start podman machine."
    }
}

Write-Host "Starting PC ROS container."
Write-Host "Workspace: $workspace"
Write-Host "ROS_DOMAIN_ID: $RosDomainId"
Write-Host "PC IP: $pcIp"

Start-PodmanMachineSafe

if (-not (podman image exists $Image)) {
    Write-Host "Building PC ROS image: $Image"
    podman build --http-proxy=false -f "$workspace/pc/arm_stack/Containerfile" -t $Image $workspace
}

podman rm -f $ContainerName 2>$null | Out-Null

$runArgs = @(
    "run",
    "--rm",
    "-d",
    "--name", $ContainerName,
    "--hostname", $ContainerName,
    "--network", "bridge",
    "-p", "127.0.0.1:8765:8765",
    "-e", "ROS_DISTRO_NAME=jazzy",
    "-e", "ROS_DOMAIN_ID=$RosDomainId",
    "-e", "ROS_LOCALHOST_ONLY=0",
    "-e", "ROS_AUTOMATIC_DISCOVERY_RANGE=SUBNET",
    "-e", "ARM_PC_IP=$pcIp",
    "-e", "ARM_HTTP_HOST=127.0.0.1",
    "-e", "DISPLAY=:0",
    "-e", "QT_X11_NO_MITSHM=1",
    "-e", "WAYLAND_DISPLAY=wayland-0",
    "-e", "XDG_RUNTIME_DIR=/mnt/wslg/runtime-dir",
    "-v", "${workspace}:${containerWorkspace}:Z",
    "-v", "/mnt/wslg:/mnt/wslg",
    "-v", "/mnt/wslg/.X11-unix:/tmp/.X11-unix",
    "-w", $containerWorkspace,
    $Image,
    "bash", "-lc", "while true; do sleep 3600; done"
)

& podman @runArgs | Out-Null

if ($Launch) {
    Write-Host "Launching RViz + debug panel inside container..."
    podman exec -d $ContainerName bash -lc "cd /workspace/arm-stack/pc/arm_stack && bash run_panel.sh >/tmp/pc_run_panel.log 2>&1"
    Write-Host "Container started and launch command dispatched."
} else {
    Write-Host "Container started."
    Write-Host "Run panel inside container with:"
    Write-Host "  podman exec -it $ContainerName bash -lc 'cd /workspace/arm-stack/pc/arm_stack && bash run_panel.sh'"
}

Write-Host "Legacy HTTP bridge is expected on http://127.0.0.1:8765 after launch."
