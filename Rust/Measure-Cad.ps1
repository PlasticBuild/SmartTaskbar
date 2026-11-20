[CmdletBinding(DefaultParameterSetName = 'Capture')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Capture')]
    [ValidateSet('Enabled', 'Paused')][string]$Condition,
    [Parameter(Mandatory, ParameterSetName = 'Capture')]
    [string]$Workflow,
    [Parameter(ParameterSetName = 'Capture')]
    [ValidateRange(10, 600)][int]$Seconds = 45,
    [Parameter(ParameterSetName = 'Capture')]
    [string]$AppDirectory = (Join-Path $PSScriptRoot '../RustInstalled'),
    [Parameter(ParameterSetName = 'Capture')]
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../bin/verification/cad'),
    [Parameter(ParameterSetName = 'Capture')]
    [switch]$Trace,
    [Parameter(Mandatory, ParameterSetName = 'Compare')]
    [string]$EnabledResult,
    [Parameter(Mandatory, ParameterSetName = 'Compare')]
    [string]$PausedResult
)

$ErrorActionPreference = 'Stop'
if ($PSCmdlet.ParameterSetName -eq 'Compare') {
    $enabled = Get-Content -LiteralPath $EnabledResult -Raw | ConvertFrom-Json
    $paused = Get-Content -LiteralPath $PausedResult -Raw | ConvertFrom-Json
    if ($enabled.Condition -ne 'Enabled' -or $paused.Condition -ne 'Paused' -or
        $enabled.Workflow -ne $paused.Workflow -or $enabled.RequestedSeconds -ne $paused.RequestedSeconds) {
        throw 'Use Enabled and Paused captures of the same workflow and duration.'
    }
    $roles = @('SmartTaskbar','Styling host','Explorer','Desktop Window Manager','FreeCAD','SOLIDWORKS')
    foreach ($role in $roles) {
        $on = ($enabled.Processes | Where-Object Role -eq $role | Measure-Object CorePercent -Sum).Sum
        $off = ($paused.Processes | Where-Object Role -eq $role | Measure-Object CorePercent -Sum).Sum
        [pscustomobject]@{
            Process = $role
            EnabledPercentOfOneCore = [Math]::Round([double]$on, 3)
            PausedPercentOfOneCore = [Math]::Round([double]$off, 3)
            Difference = [Math]::Round(([double]$on - [double]$off), 3)
        }
    }
    return
}

$folder = (Resolve-Path -LiteralPath $AppDirectory).Path
$exe = Join-Path $folder 'smarttaskbar-rs.exe'
$style = Join-Path $folder 'Windhawk/windhawk.exe'
if (!(Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Point AppDirectory at the installed SmartTaskbar build.' }
$marker = Join-Path $folder 'settings.json.paused'
$session = (Get-Process -Id $PID).SessionId
$roles = @{
    'smarttaskbar-rs' = 'SmartTaskbar'; windhawk = 'Styling host'; explorer = 'Explorer'
    dwm = 'Desktop Window Manager'; FreeCAD = 'FreeCAD'; SLDWORKS = 'SOLIDWORKS'
}
function Read-Processes {
    Get-Process -Name @($roles.Keys) -ErrorAction SilentlyContinue |
        Where-Object { $_.SessionId -eq $session -and $null -ne $_.CPU } |
        Where-Object { $_.ProcessName -notin 'smarttaskbar-rs','windhawk' -or $_.Path -in $exe,$style } |
        ForEach-Object {
            [pscustomobject]@{
                Id = $_.Id
                Started = $_.StartTime.ToUniversalTime().Ticks
                Role = $roles[$_.ProcessName]
                CpuMs = 1000 * $_.CPU
                PrivateMiB = $_.PrivateMemorySize64 / 1MB
            }
        }
}

$null = New-Item -ItemType Directory -Path $OutputDirectory -Force
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$base = Join-Path $output "$stamp-$Condition"
$captureId = "SmartTaskbar-$PID-$stamp"
$recording = $false
try {
    if ($Trace) {
        $wpr = Join-Path $env:SystemRoot 'System32/wpr.exe'
        & $wpr -start CPU -start GPU -start DesktopComposition -filemode -recordtempto $output -instancename $captureId
        if ($LASTEXITCODE) { throw 'Windows Performance Recorder could not start the capture.' }
        $recording = $true
    }
    $before = @(Read-Processes)
    $running = [bool]($before | Where-Object Role -eq 'SmartTaskbar')
    if (!(($before | Where-Object Role -in 'FreeCAD','SOLIDWORKS'))) { throw 'Open FreeCAD or SOLIDWORKS first.' }
    $paused = Test-Path -LiteralPath $marker -PathType Leaf
    if (($Condition -eq 'Enabled' -and (!$running -or $paused)) -or
        ($Condition -eq 'Paused' -and ($running -or !$paused -or ($before | Where-Object Role -eq 'Styling host')))) {
        throw 'Set the selected Pause or Resume state and close SmartTaskbar Settings before capturing.'
    }
    Write-Host "Capture started ($Condition). Run the same CAD actions for $Seconds seconds."
    $timer = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds $Seconds
    $after = @(Read-Processes)
    $elapsed = $timer.Elapsed.TotalSeconds
    if ($paused -ne (Test-Path -LiteralPath $marker -PathType Leaf)) {
        throw 'The SmartTaskbar pause state changed during capture.'
    }
    $rows = foreach ($sample in $before) {
        $end = $after | Where-Object { $_.Id -eq $sample.Id -and $_.Started -eq $sample.Started }
        if (!$end) { throw "$($sample.Role) exited or restarted during capture. Repeat this run." }
        $cpu = $end.CpuMs - $sample.CpuMs
        [pscustomobject]@{
            Role = $sample.Role
            CpuMs = [Math]::Round($cpu, 2)
            CorePercent = [Math]::Round($cpu / $elapsed / 10, 4)
            EndPrivateMiB = [Math]::Round($end.PrivateMiB, 2)
        }
    }
    if ($recording) {
        & $wpr -stop "$base.etl" -skipPdbGen -compress -instancename $captureId
        if ($LASTEXITCODE) { throw 'Windows Performance Recorder could not save the capture.' }
        $recording = $false
    }
    [pscustomobject]@{
        Workflow = $Workflow
        Condition = $Condition
        RequestedSeconds = $Seconds
        ElapsedSeconds = $elapsed
        Processes = @($rows)
        Trace = if ($Trace) { "$base.etl" } else { $null }
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath "$base.json" -Encoding utf8
    Get-Item -LiteralPath "$base.json"
} finally {
    if ($recording) { & $wpr -cancel -instancename $captureId }
}
