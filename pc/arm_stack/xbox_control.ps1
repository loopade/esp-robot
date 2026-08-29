param(
    [string]$BridgeHost = "127.0.0.1",
    [int]$Port = 8765,
    [double]$PollHz = 25.0,
    [double]$SpeedScale = 1.0,
    [double]$PrecisionScale = 0.35
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class XInputNative {
    [StructLayout(LayoutKind.Sequential)]
    public struct XINPUT_GAMEPAD {
        public ushort wButtons;
        public byte bLeftTrigger;
        public byte bRightTrigger;
        public short sThumbLX;
        public short sThumbLY;
        public short sThumbRX;
        public short sThumbRY;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct XINPUT_STATE {
        public uint dwPacketNumber;
        public XINPUT_GAMEPAD Gamepad;
    }

    [DllImport("xinput1_4.dll", EntryPoint = "XInputGetState")]
    public static extern uint XInputGetState(int dwUserIndex, out XINPUT_STATE pState);
}
"@

$ButtonMask = @{
    DPadUp     = 0x0001
    DPadDown   = 0x0002
    DPadLeft   = 0x0004
    DPadRight  = 0x0008
    Start      = 0x0010
    Back       = 0x0020
    LeftThumb  = 0x0040
    RightThumb = 0x0080
    LB         = 0x0100
    RB         = 0x0200
    A          = 0x1000
    B          = 0x2000
    X          = 0x4000
    Y          = 0x8000
}

$BaseJointRatesDegPerSec = @{
    0 = 26.0
    1 = 18.0
    2 = 18.0
    3 = 22.0
    4 = 34.0
    5 = 28.0
}

$JointNames = @("base", "shoulder", "elbow", "wrist_pitch", "wrist_rotate", "gripper")
$TargetMinDeg = @(-120.0, -180.0, -180.0, -180.0, -180.0, -180.0)
$TargetMaxDeg = @(120.0, 180.0, 180.0, 180.0, 180.0, 180.0)
$JointSoftLimitActive = @($false, $false, $false, $false, $false, $false)

function Invoke-ArmGet {
    param([string]$Path)

    Invoke-RestMethod `
        -Uri ("http://{0}:{1}{2}" -f $BridgeHost, $Port, $Path) `
        -Method Get `
        -TimeoutSec 2
}

function Invoke-ArmPost {
    param(
        [string]$Path,
        [hashtable]$Body
    )

    Invoke-RestMethod `
        -Uri ("http://{0}:{1}{2}" -f $BridgeHost, $Port, $Path) `
        -Method Post `
        -TimeoutSec 2 `
        -ContentType "application/json" `
        -Body ($Body | ConvertTo-Json -Compress)
}

function Normalize-Stick {
    param(
        [int]$Value,
        [double]$Deadzone = 0.18,
        [double]$Exponent = 1.7
    )

    $normalized = [Math]::Max(-1.0, [Math]::Min(1.0, [double]$Value / 32767.0))
    if ([Math]::Abs($normalized) -lt $Deadzone) {
        return 0.0
    }

    $magnitude = ([Math]::Abs($normalized) - $Deadzone) / (1.0 - $Deadzone)
    $shaped = [Math]::Pow($magnitude, $Exponent)
    return [Math]::Sign($normalized) * $shaped
}

function Normalize-Trigger {
    param(
        [int]$Value,
        [double]$Deadzone = 0.12,
        [double]$Exponent = 1.4
    )

    $normalized = [Math]::Max(0.0, [Math]::Min(1.0, [double]$Value / 255.0))
    if ($normalized -lt $Deadzone) {
        return 0.0
    }

    $magnitude = ($normalized - $Deadzone) / (1.0 - $Deadzone)
    return [Math]::Pow($magnitude, $Exponent)
}

function Test-ButtonPressed {
    param(
        [int]$Buttons,
        [int]$Mask
    )

    return (($Buttons -band $Mask) -ne 0)
}

function Test-ButtonEdge {
    param(
        [int]$Buttons,
        [int]$PreviousButtons,
        [int]$Mask
    )

    $isPressed = Test-ButtonPressed -Buttons $Buttons -Mask $Mask
    $wasPressed = Test-ButtonPressed -Buttons $PreviousButtons -Mask $Mask
    return ($isPressed -and -not $wasPressed)
}

function Clamp-Range {
    param(
        [double]$Value,
        [double]$Minimum,
        [double]$Maximum
    )

    return [Math]::Max($Minimum, [Math]::Min($Maximum, $Value))
}

function Update-TargetLimitsFromPayload {
    param($Payload)

    if ($null -eq $Payload) {
        return
    }

    $hasTargetMin = $null -ne ($Payload.PSObject.Properties["target_min_deg"])
    $hasTargetMax = $null -ne ($Payload.PSObject.Properties["target_max_deg"])
    if (
        $hasTargetMin -and
        $hasTargetMax -and
        $Payload.target_min_deg.Count -eq $JointNames.Count -and
        $Payload.target_max_deg.Count -eq $JointNames.Count
    ) {
        $script:TargetMinDeg = @($Payload.target_min_deg | ForEach-Object { [double]$_ })
        $script:TargetMaxDeg = @($Payload.target_max_deg | ForEach-Object { [double]$_ })
    }
}

function Clamp-JointPositions {
    param([double[]]$Positions)

    for ($index = 0; $index -lt $Positions.Length; $index++) {
        $minimum = [double]$TargetMinDeg[$index]
        $maximum = [double]$TargetMaxDeg[$index]
        $original = [double]$Positions[$index]
        $Positions[$index] = Clamp-Range -Value $original -Minimum $minimum -Maximum $maximum

        $isClamped = [Math]::Abs($Positions[$index] - $original) -ge 0.001
        if ($isClamped -and -not $JointSoftLimitActive[$index]) {
            $JointSoftLimitActive[$index] = $true
            Write-Host ("LIMIT {0} -> clamped to {1:N1} deg" -f $JointNames[$index], $Positions[$index])
        } elseif ((-not $isClamped) -and $JointSoftLimitActive[$index]) {
            $JointSoftLimitActive[$index] = $false
            Write-Host ("LIMIT {0} -> released" -f $JointNames[$index])
        }
    }
}

function Get-EffectiveSpeedScale {
    param(
        [double]$BaseSpeedScale,
        [bool]$PrecisionMode,
        [double]$PrecisionScale
    )

    $effective = $BaseSpeedScale
    if ($PrecisionMode) {
        $effective *= $PrecisionScale
    }

    return Clamp-Range -Value $effective -Minimum 0.1 -Maximum 3.0
}

function Get-InputSpeedFactor {
    param(
        [double]$BaseSpeedScale,
        [bool]$PrecisionMode,
        [double]$PrecisionScale
    )

    $factor = [Math]::Sqrt((Clamp-Range -Value $BaseSpeedScale -Minimum 0.2 -Maximum 2.0))
    if ($PrecisionMode) {
        $factor *= $PrecisionScale
    }

    return $factor
}

function Publish-SpeedScale {
    param(
        [double]$BaseSpeedScale,
        [bool]$PrecisionMode,
        [double]$PrecisionScale
    )

    $effectiveSpeed = Get-EffectiveSpeedScale -BaseSpeedScale $BaseSpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
    $roundedSpeed = [Math]::Round($effectiveSpeed, 2)
    $null = Invoke-ArmPost -Path "/motion_config" -Body @{ speed_scale = $roundedSpeed }
    Write-Host ("SEND /motion_config -> speed_scale={0}" -f $roundedSpeed)
    return $roundedSpeed
}

function Get-CurrentJointState {
    $jointState = Invoke-ArmGet -Path "/joint_state"
    return [double[]]@($jointState.positions_deg | ForEach-Object { [double]$_ })
}

function Get-TargetState {
    $payload = Invoke-ArmGet -Path "/target_state"
    Update-TargetLimitsFromPayload -Payload $payload
    return @{
        positions_deg = [double[]]@($payload.positions_deg | ForEach-Object { [double]$_ })
        target_min_deg = [double[]]@($TargetMinDeg)
        target_max_deg = [double[]]@($TargetMaxDeg)
    }
}

function Test-GoalChanged {
    param(
        [double[]]$CurrentPositions,
        [double[]]$PreviousPositions,
        [double]$ToleranceDeg = 0.08
    )

    if ($null -eq $PreviousPositions) {
        return $true
    }

    for ($index = 0; $index -lt $CurrentPositions.Length; $index++) {
        if ([Math]::Abs($CurrentPositions[$index] - $PreviousPositions[$index]) -ge $ToleranceDeg) {
            return $true
        }
    }

    return $false
}

function Send-Goal {
    param([double[]]$Positions)

    Clamp-JointPositions -Positions $Positions
    $payload = @{
        positions_deg = @($Positions | ForEach-Object { [Math]::Round($_, 2) })
    }
    Write-Host ("SEND /goal -> {0}" -f ($payload.positions_deg -join ", "))
    $response = Invoke-ArmPost -Path "/goal" -Body $payload
    if ($response.positions_deg -is [System.Collections.IEnumerable]) {
        return [double[]]@($response.positions_deg | ForEach-Object { [double]$_ })
    }
    return [double[]]@($Positions)
}

function Write-Mapping {
    param(
        [double]$BaseSpeedScale,
        [bool]$PrecisionMode,
        [double]$EffectiveSpeedScale
    )

    $modeLabel = if ($PrecisionMode) { "precision" } else { "normal" }

    Write-Host "Xbox controller script started. Press Ctrl+C to exit."
    Write-Host ("Mode: {0}, base speed {1:N1}x, applied speed {2:N2}x" -f $modeLabel, $BaseSpeedScale, $EffectiveSpeedScale)
    Write-Host ("Shared target limits: base [{0}, {1}] deg" -f $TargetMinDeg[0], $TargetMaxDeg[0])
    Write-Host "Mapping:"
    Write-Host "  D-pad left / right: base target slider step"
    Write-Host "  Left stick Y: shoulder"
    Write-Host "  Right stick Y: elbow"
    Write-Host "  Right stick X: wrist pitch"
    Write-Host "  LB / RB: wrist rotate"
    Write-Host "  LT / RT: gripper open / close"
    Write-Host "  D-pad up / down: base speed"
    Write-Host "  X: toggle precision mode"
    Write-Host "  A: sync current pose into shared target"
    Write-Host "  B: send current target again"
    Write-Host "  Y: reset speed to 1.0x"
}

try {
    $targetState = Get-TargetState
    $positions = [double[]]@($targetState.positions_deg)
} catch {
    Write-Warning ("Failed to read shared target state, fallback to joint_state: {0}" -f $_.Exception.Message)
    $positions = Get-CurrentJointState
}

Clamp-JointPositions -Positions $positions

$SpeedScale = Clamp-Range -Value $SpeedScale -Minimum 0.2 -Maximum 2.0
$PrecisionScale = Clamp-Range -Value $PrecisionScale -Minimum 0.2 -Maximum 0.8
$PrecisionMode = $false
$EffectiveSpeedScale = Publish-SpeedScale -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
$InputSpeedFactor = Get-InputSpeedFactor -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
$LastSentPositions = $null
$LastButtons = 0
$ControllerConnected = $false
$LastDpadAdjust = [datetime]::MinValue
$LastRemoteTargetSync = [datetime]::MinValue
$BaseStepDirection = 0
$BaseFirstRepeatDelaySec = 0.18
$BaseRepeatIntervalSec = 0.05
$BaseSliderStepDeg = 1.0
$BasePressStartedAt = [datetime]::MinValue
$BaseLastStepAt = [datetime]::MinValue
$IntervalMs = [Math]::Max(20, [int](1000.0 / $PollHz))

Write-Mapping -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -EffectiveSpeedScale $EffectiveSpeedScale

while ($true) {
    $state = New-Object XInputNative+XINPUT_STATE
    $result = [XInputNative]::XInputGetState(0, [ref]$state)

    if ($result -ne 0) {
        if ($ControllerConnected) {
            Write-Host "Xbox controller disconnected. Waiting for reconnect..."
            $ControllerConnected = $false
        }
        Start-Sleep -Milliseconds 250
        continue
    }

    if (-not $ControllerConnected) {
        Write-Host "Xbox controller connected."
        $ControllerConnected = $true
    }

    $gamepad = $state.Gamepad
    $buttons = [int]$gamepad.wButtons
    $dt = $IntervalMs / 1000.0
    $now = Get-Date

    if (Test-ButtonEdge -Buttons $buttons -PreviousButtons $LastButtons -Mask $ButtonMask.X) {
        $PrecisionMode = -not $PrecisionMode
        try {
            $EffectiveSpeedScale = Publish-SpeedScale -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
            $InputSpeedFactor = Get-InputSpeedFactor -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
            $modeLabel = if ($PrecisionMode) { "precision" } else { "normal" }
            Write-Host ("Switched to {0} mode, applied speed {1:N2}x" -f $modeLabel, $EffectiveSpeedScale)
        } catch {
            Write-Warning ("Failed to switch precision mode: {0}" -f $_.Exception.Message)
        }
    }

    if (Test-ButtonEdge -Buttons $buttons -PreviousButtons $LastButtons -Mask $ButtonMask.Y) {
        $SpeedScale = 1.0
        try {
            $EffectiveSpeedScale = Publish-SpeedScale -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
            $InputSpeedFactor = Get-InputSpeedFactor -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
            Write-Host ("Base speed reset to 1.0x, applied speed {0:N2}x" -f $EffectiveSpeedScale)
        } catch {
            Write-Warning ("Failed to reset base speed: {0}" -f $_.Exception.Message)
        }
    }

    if (Test-ButtonEdge -Buttons $buttons -PreviousButtons $LastButtons -Mask $ButtonMask.A) {
        try {
            $positions = Get-CurrentJointState
            Clamp-JointPositions -Positions $positions
            $positions = Send-Goal -Positions $positions
            $LastSentPositions = [double[]]@($positions)
            Write-Host "Synced current hardware pose into shared target."
        } catch {
            Write-Warning ("Failed to sync current pose: {0}" -f $_.Exception.Message)
        }
    }

    if (Test-ButtonEdge -Buttons $buttons -PreviousButtons $LastButtons -Mask $ButtonMask.B) {
        try {
            $positions = Send-Goal -Positions $positions
            $LastSentPositions = [double[]]@($positions)
            Write-Host "Re-sent current shared target."
        } catch {
            Write-Warning ("Failed to resend current target: {0}" -f $_.Exception.Message)
        }
    }

    $dpadUpPressed = Test-ButtonPressed -Buttons $buttons -Mask $ButtonMask.DPadUp
    $dpadDownPressed = Test-ButtonPressed -Buttons $buttons -Mask $ButtonMask.DPadDown
    if (($dpadUpPressed -or $dpadDownPressed) -and ($now - $LastDpadAdjust).TotalMilliseconds -ge 250) {
        if ($dpadUpPressed) {
            $SpeedScale = Clamp-Range -Value ($SpeedScale + 0.1) -Minimum 0.2 -Maximum 2.0
        } else {
            $SpeedScale = Clamp-Range -Value ($SpeedScale - 0.1) -Minimum 0.2 -Maximum 2.0
        }

        try {
            $EffectiveSpeedScale = Publish-SpeedScale -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
            $InputSpeedFactor = Get-InputSpeedFactor -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
            Write-Host ("Base speed: {0:N1}x, applied speed: {1:N2}x" -f $SpeedScale, $EffectiveSpeedScale)
            $LastDpadAdjust = $now
        } catch {
            Write-Warning ("Failed to update speed: {0}" -f $_.Exception.Message)
        }
    }

    $ly = Normalize-Stick -Value $gamepad.sThumbLY
    $rx = Normalize-Stick -Value $gamepad.sThumbRX
    $ry = Normalize-Stick -Value $gamepad.sThumbRY
    $lt = Normalize-Trigger -Value $gamepad.bLeftTrigger
    $rt = Normalize-Trigger -Value $gamepad.bRightTrigger
    $dpadLeftPressed = Test-ButtonPressed -Buttons $buttons -Mask $ButtonMask.DPadLeft
    $dpadRightPressed = Test-ButtonPressed -Buttons $buttons -Mask $ButtonMask.DPadRight
    $baseDirection = 0
    if ($dpadLeftPressed -and -not $dpadRightPressed) {
        $baseDirection = -1
    } elseif ($dpadRightPressed -and -not $dpadLeftPressed) {
        $baseDirection = 1
    }

    $hadInput = $false
    if ($baseDirection -eq 0) {
        $BaseStepDirection = 0
    } else {
        $hadInput = $true
        $shouldStepBase = $false
        if ($BaseStepDirection -ne $baseDirection) {
            $BaseStepDirection = $baseDirection
            $BasePressStartedAt = $now
            $BaseLastStepAt = $now
            $shouldStepBase = $true
        } else {
            $heldSeconds = ($now - $BasePressStartedAt).TotalSeconds
            $elapsedSinceStep = ($now - $BaseLastStepAt).TotalSeconds
            if ($heldSeconds -ge $BaseFirstRepeatDelaySec -and $elapsedSinceStep -ge $BaseRepeatIntervalSec) {
                $BaseLastStepAt = $now
                $shouldStepBase = $true
            }
        }

        if ($shouldStepBase) {
            $positions[0] += $baseDirection * $BaseSliderStepDeg
        }
    }

    $positions[1] += (-$ly) * $BaseJointRatesDegPerSec[1] * $InputSpeedFactor * $dt
    $positions[2] += (-$ry) * $BaseJointRatesDegPerSec[2] * $InputSpeedFactor * $dt
    $positions[3] += $rx * $BaseJointRatesDegPerSec[3] * $InputSpeedFactor * $dt

    if ([Math]::Abs($ly) -ge 0.001 -or [Math]::Abs($ry) -ge 0.001 -or [Math]::Abs($rx) -ge 0.001) {
        $hadInput = $true
    }

    $lbPressed = Test-ButtonPressed -Buttons $buttons -Mask $ButtonMask.LB
    $rbPressed = Test-ButtonPressed -Buttons $buttons -Mask $ButtonMask.RB
    if ($lbPressed -and -not $rbPressed) {
        $positions[4] -= $BaseJointRatesDegPerSec[4] * $InputSpeedFactor * $dt
        $hadInput = $true
    } elseif ($rbPressed -and -not $lbPressed) {
        $positions[4] += $BaseJointRatesDegPerSec[4] * $InputSpeedFactor * $dt
        $hadInput = $true
    }

    if ([Math]::Abs($lt) -ge 0.001 -or [Math]::Abs($rt) -ge 0.001) {
        $positions[5] += ($lt - $rt) * $BaseJointRatesDegPerSec[5] * $InputSpeedFactor * $dt
        $hadInput = $true
    }

    Clamp-JointPositions -Positions $positions

    if (Test-GoalChanged -CurrentPositions $positions -PreviousPositions $LastSentPositions) {
        try {
            $positions = Send-Goal -Positions $positions
            $LastSentPositions = [double[]]@($positions)
            $LastRemoteTargetSync = $now
        } catch {
            Write-Warning ("Failed to send goal: {0}" -f $_.Exception.Message)
        }
    } elseif ((-not $hadInput) -and ($now - $LastRemoteTargetSync).TotalMilliseconds -ge 400) {
        try {
            $targetState = Get-TargetState
            $positions = [double[]]@($targetState.positions_deg)
            Clamp-JointPositions -Positions $positions
            $LastSentPositions = [double[]]@($positions)
            $LastRemoteTargetSync = $now
        } catch {
            Write-Warning ("Failed to refresh shared target state: {0}" -f $_.Exception.Message)
        }
    }

    $LastButtons = $buttons
    Start-Sleep -Milliseconds $IntervalMs
}
