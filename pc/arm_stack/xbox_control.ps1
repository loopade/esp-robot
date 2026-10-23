param(
    [string]$ControlCenterUrl = "http://127.0.0.1:8080",
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

$script:ArmHost = $null

$BaseJointRatesDegPerSec = @{
    0 = 26.0
    1 = 18.0
    2 = 18.0
    3 = 22.0
    4 = 34.0
    5 = 28.0
}

$JointNames = @("base", "shoulder", "elbow", "wrist_pitch", "wrist_rotate")
$TargetMinDeg = @(-90.0, -90.0, -90.0, -90.0, -90.0)
$TargetMaxDeg = @(90.0, 90.0, 90.0, 90.0, 90.0)
$JointSoftLimitActive = @($false, $false, $false, $false, $false)
$script:positions = [double[]]@(0, 0, 0, 0, 0)

function Get-ControlState {
    Invoke-RestMethod -Uri "$ControlCenterUrl/api/state" -Method Get -TimeoutSec 2
}

function Invoke-ControlPost {
    param([string]$Path, [hashtable]$Body)
    Invoke-RestMethod -Uri "$ControlCenterUrl$Path" -Method Post -TimeoutSec 2 `
        -ContentType "application/json" -Body ($Body | ConvertTo-Json -Compress)
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
    return $roundedSpeed
}

function Get-CurrentJointState {
    $state = Get-ControlState
    $angles = $state.telemetry.current_deg
    if ($angles -and $angles.Count -ge 5) { return [double[]]@($angles | Select-Object -First 5 | ForEach-Object { [double]$_ }) }
    return [double[]]@($script:positions)
}

function Get-TargetState {
    $state = Get-ControlState
    $arm = @($state.devices | Where-Object { $_.device -eq 'arm' -or $_.control_port -eq 8888 } | Select-Object -First 1)
    if (-not $arm -or -not $arm[0].peer) { throw 'No arm discovered by control center yet.' }
    $script:ArmHost = [string]$arm[0].peer
    $payload = $state.telemetry
    if ($payload) {
        if ($payload.min_angle_deg -and $payload.max_angle_deg) {
            $script:TargetMinDeg = @($payload.min_angle_deg | ForEach-Object { [double]$_ })
            $script:TargetMaxDeg = @($payload.max_angle_deg | ForEach-Object { [double]$_ })
        }
        if ($payload.target_deg -and $payload.target_deg.Count -ge 5) {
            $script:positions = [double[]]@($payload.target_deg | Select-Object -First 5 | ForEach-Object { [double]$_ })
        }
    }
    if ($script:positions.Count -ne 5) {
        throw ("Expected 5 arm joint positions, received {0}." -f $script:positions.Count)
    }
    return @{ positions_deg = [double[]]@($script:positions); target_min_deg = $TargetMinDeg; target_max_deg = $TargetMaxDeg }
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
    $radians = @($Positions | Select-Object -First 5 | ForEach-Object { [Math]::Round($_ * [Math]::PI / 180.0, 5) })
    $null = Invoke-ControlPost -Path "/api/arm" -Body @{ host = $script:ArmHost; position = $radians; speed_scale = $script:EffectiveSpeedScale }
    return [double[]]@($Positions)
}

function Send-ChassisDrive {
    param(
        [double]$Left,
        [double]$Right,
        [bool]$Armed
    )

    $null = Invoke-ControlPost -Path "/api/control" -Body @{
        left = [int][Math]::Round($Left); right = [int][Math]::Round($Right); armed = $Armed
    }
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
    Write-Host "  Left stick Y / X: chassis forward / steering (release to stop)"
    Write-Host "  D-pad left / right: arm base joint"
    Write-Host "  D-pad up / down: shoulder"
    Write-Host "  Right stick Y: elbow"
    Write-Host "  Right stick X: wrist pitch"
    Write-Host "  LB / RB: wrist rotate"
    Write-Host "  X: toggle precision mode"
    Write-Host "  A: sync current pose into shared target"
    Write-Host "  B: send current target again"
    Write-Host "  Y: reset speed to 1.0x"
}

$SpeedScale = Clamp-Range -Value $SpeedScale -Minimum 0.2 -Maximum 2.0
$PrecisionScale = Clamp-Range -Value $PrecisionScale -Minimum 0.2 -Maximum 0.8
$PrecisionMode = $false
$ArmAvailable = $false
$ArmRetryAt = [datetime]::MinValue
$positions = [double[]]@(0, 0, 0, 0, 0)
$LastSentPositions = $null
$EffectiveSpeedScale = Get-EffectiveSpeedScale -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
try {
    $targetState = Get-TargetState
    $positions = [double[]]@($targetState.positions_deg)
    Clamp-JointPositions -Positions $positions
    $EffectiveSpeedScale = Publish-SpeedScale -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
    $ArmAvailable = $true
} catch {
    Write-Warning ("Control center unavailable at {0}; chassis/arm controls will retry: {1}" -f $ControlCenterUrl, $_.Exception.Message)
}
$InputSpeedFactor = Get-InputSpeedFactor -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
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
$ShoulderStepDirection = 0
$ShoulderPressStartedAt = [datetime]::MinValue
$ShoulderLastStepAt = [datetime]::MinValue
$IntervalMs = [Math]::Max(20, [int](1000.0 / $PollHz))

Write-Mapping -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -EffectiveSpeedScale $EffectiveSpeedScale

try {
while ($true) {
    $state = New-Object XInputNative+XINPUT_STATE
    $result = [XInputNative]::XInputGetState(0, [ref]$state)

    if ($result -ne 0) {
        if ($ControllerConnected) {
            Write-Host "Xbox controller disconnected. Waiting for reconnect..."
            $ControllerConnected = $false
        }
        try {
            Send-ChassisDrive -Left 0 -Right 0 -Armed $false
        } catch {
            Write-Warning ("Failed to stop chassis after controller disconnect: {0}" -f $_.Exception.Message)
        }
        $LastButtons = 0
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

    if (-not $ArmAvailable -and $now -ge $ArmRetryAt) {
        $ArmRetryAt = $now.AddSeconds(2)
        try {
            $targetState = Get-TargetState
            $positions = [double[]]@($targetState.positions_deg)
            Clamp-JointPositions -Positions $positions
            $EffectiveSpeedScale = Publish-SpeedScale -BaseSpeedScale $SpeedScale -PrecisionMode $PrecisionMode -PrecisionScale $PrecisionScale
            $LastSentPositions = [double[]]@($positions)
            $LastRemoteTargetSync = $now
            $ArmAvailable = $true
            Write-Host "Arm bridge connected; arm control enabled."
        } catch {
            Write-Host ("Waiting for discovered arm through control center... {0}" -f $_.Exception.Message)
        }
    }

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

    if ($ArmAvailable -and (Test-ButtonEdge -Buttons $buttons -PreviousButtons $LastButtons -Mask $ButtonMask.A)) {
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

    if ($ArmAvailable -and (Test-ButtonEdge -Buttons $buttons -PreviousButtons $LastButtons -Mask $ButtonMask.B)) {
        try {
            $positions = Send-Goal -Positions $positions
            $LastSentPositions = [double[]]@($positions)
            Write-Host "Re-sent current shared target."
        } catch {
            Write-Warning ("Failed to resend current target: {0}" -f $_.Exception.Message)
        }
    }

    $ly = Normalize-Stick -Value $gamepad.sThumbLY
    $lx = Normalize-Stick -Value $gamepad.sThumbLX
    $rx = Normalize-Stick -Value $gamepad.sThumbRX
    $ry = Normalize-Stick -Value $gamepad.sThumbRY
    $lt = Normalize-Trigger -Value $gamepad.bLeftTrigger
    $rt = Normalize-Trigger -Value $gamepad.bRightTrigger

    # Arcade drive: left stick Y is throttle and X is steering.
    $throttle = -$ly
    $turn = $lx
    $leftDrive = $throttle + $turn
    $rightDrive = $throttle - $turn
    $drivePeak = [Math]::Max([Math]::Abs($leftDrive), [Math]::Abs($rightDrive))
    if ($drivePeak -gt 1.0) {
        $leftDrive /= $drivePeak
        $rightDrive /= $drivePeak
    }
    try {
        $driveArmed = ([Math]::Abs($throttle) -ge 0.01 -or [Math]::Abs($turn) -ge 0.01)
        Send-ChassisDrive -Left ($leftDrive * 1000) -Right ($rightDrive * 1000) -Armed $driveArmed
    } catch {
        Write-Warning ("Failed to send chassis command: {0}" -f $_.Exception.Message)
    }

    if ($ArmAvailable) {
    $dpadUpPressed = Test-ButtonPressed -Buttons $buttons -Mask $ButtonMask.DPadUp
    $dpadDownPressed = Test-ButtonPressed -Buttons $buttons -Mask $ButtonMask.DPadDown
    $hadInput = $false
    $shoulderDirection = 0
    if ($dpadUpPressed -and -not $dpadDownPressed) {
        $shoulderDirection = 1
    } elseif ($dpadDownPressed -and -not $dpadUpPressed) {
        $shoulderDirection = -1
    }
    if ($shoulderDirection -eq 0) {
        $ShoulderStepDirection = 0
    } else {
        $hadInput = $true
        $shouldStepShoulder = $false
        if ($ShoulderStepDirection -ne $shoulderDirection) {
            $ShoulderStepDirection = $shoulderDirection
            $ShoulderPressStartedAt = $now
            $ShoulderLastStepAt = $now
            $shouldStepShoulder = $true
        } else {
            $heldSeconds = ($now - $ShoulderPressStartedAt).TotalSeconds
            $elapsedSinceStep = ($now - $ShoulderLastStepAt).TotalSeconds
            if ($heldSeconds -ge $BaseFirstRepeatDelaySec -and $elapsedSinceStep -ge $BaseRepeatIntervalSec) {
                $ShoulderLastStepAt = $now
                $shouldStepShoulder = $true
            }
        }
        if ($shouldStepShoulder) {
            $positions[1] += $shoulderDirection * $BaseSliderStepDeg
        }
    }

    $dpadLeftPressed = Test-ButtonPressed -Buttons $buttons -Mask $ButtonMask.DPadLeft
    $dpadRightPressed = Test-ButtonPressed -Buttons $buttons -Mask $ButtonMask.DPadRight
    $baseDirection = 0
    if ($dpadLeftPressed -and -not $dpadRightPressed) {
        $baseDirection = -1
    } elseif ($dpadRightPressed -and -not $dpadLeftPressed) {
        $baseDirection = 1
    }

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

    $positions[2] += (-$ry) * $BaseJointRatesDegPerSec[2] * $InputSpeedFactor * $dt
    $positions[3] += $rx * $BaseJointRatesDegPerSec[3] * $InputSpeedFactor * $dt

    if ([Math]::Abs($ry) -ge 0.001 -or [Math]::Abs($rx) -ge 0.001) {
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
    }

    $LastButtons = $buttons
    Start-Sleep -Milliseconds $IntervalMs
}
} finally {
    try {
        for ($i = 0; $i -lt 10; $i++) {
            Send-ChassisDrive -Left 0 -Right 0 -Armed $false
            Start-Sleep -Milliseconds 20
        }
    } catch {
        Write-Warning ("Failed to send chassis stop command: {0}" -f $_.Exception.Message)
    }
}
