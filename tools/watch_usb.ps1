param(
    [ValidateRange(1, 86400)]
    [int]$Seconds = 600,
    [ValidateRange(0.5, 60)]
    [double]$PollSeconds = 1,
    [string]$OutputPath
)

# Observe Windows enumeration only: no recording, debugger, reset or power changes.
$ErrorActionPreference = 'Stop'
if (-not $OutputPath) {
    $OutputPath = Join-Path (Split-Path $PSScriptRoot -Parent) (
        'captures/usb-watch-{0}.jsonl' -f (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
}
$OutputPath = [IO.Path]::GetFullPath($OutputPath)
$null = New-Item -ItemType Directory -Force -Path (Split-Path $OutputPath -Parent)
# Never overwrite an earlier observation.
$file = [IO.File]::Open($OutputPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
$writer = [IO.StreamWriter]::new($file, [Text.UTF8Encoding]::new($false))
$writer.AutoFlush = $true

function Write-Observation($Value) {
    $line = ConvertTo-Json -InputObject $Value -Depth 8 -Compress
    $writer.WriteLine($line)
    Write-Host $line
}

$clock = [Diagnostics.Stopwatch]::StartNew()
$previous = $null
$polls = 0
$trackedParents = @()
$previousMicIds = @()
try {
    Write-Observation ([ordered]@{
        time = Get-Date -Format o
        event = 'start'
        seconds = $Seconds
        poll_seconds = $PollSeconds
        scope = 'Windows present-device polling; brief transitions between polls can be missed. No firmware/CPU/hardware diagnosis.'
    })
    while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
        # A failed query terminates the run rather than reporting a false removal.
        $present = @(Get-PnpDevice -PresentOnly -ErrorAction Stop)
        $mic = @($present | Where-Object InstanceId -match '^USB\\VID_C0DE&PID_CAFE\\')
        foreach ($device in $mic) {
            if ($device.InstanceId -in $previousMicIds) { continue }
            $parentId = $device.InstanceId
            for ($depth = 0; $depth -lt 8; $depth++) {
                $parent = Get-PnpDeviceProperty -InstanceId $parentId `
                    -KeyName 'DEVPKEY_Device_Parent' -ErrorAction Stop
                if (-not $parent.Data) { break }
                $parentId = [string]$parent.Data
                # Track the USB hubs, including the root hub, after removal too.
                if ($parentId -notmatch '^USB\\') { break }
                if ($parentId -notin $trackedParents) { $trackedParents += $parentId }
            }
        }
        $previousMicIds = @($mic | ForEach-Object InstanceId)
        $devices = @($present | Where-Object {
            $_.InstanceId -match '^USB\\VID_C0DE&PID_CAFE|^USB\\VID_0483&PID_3748' -or
            $_.FriendlyName -match 'STM32F4 USB Microphone' -or
            $_.InstanceId -in $trackedParents
        } | Sort-Object InstanceId | Select-Object Status, Class, FriendlyName, InstanceId)
        $state = ConvertTo-Json -InputObject $devices -Depth 4 -Compress
        $polls++
        if ($state -ne $previous) {
            Write-Observation ([ordered]@{
                time = Get-Date -Format o
                event = if ($null -eq $previous) { 'baseline' } else { 'change' }
                devices = $devices
                tracked_usb_parents = $trackedParents
            })
            $previous = $state
        }
        $remaining = $Seconds - $clock.Elapsed.TotalSeconds
        if ($remaining -gt 0) {
            Start-Sleep -Milliseconds ([int](1000 * [Math]::Min($PollSeconds, $remaining)))
        }
    }
    Write-Observation ([ordered]@{
        time = Get-Date -Format o
        event = 'complete'
        polls = $polls
        elapsed_seconds = $clock.Elapsed.TotalSeconds
    })
}
catch {
    Write-Observation ([ordered]@{ time = Get-Date -Format o; event = 'error'; message = $_.Exception.Message })
    throw
}
finally {
    $writer.Dispose()
    Write-Host "USB observations: $OutputPath"
}
