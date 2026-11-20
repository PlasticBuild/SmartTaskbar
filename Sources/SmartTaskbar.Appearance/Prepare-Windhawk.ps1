param([Parameter(Mandatory)][string]$OutputDirectory)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
$portable = Join-Path $output 'Windhawk'
if (Test-Path -LiteralPath $portable) {
    throw "A Windhawk directory already exists in $output."
}
if (Get-Process -Name windhawk -ErrorAction SilentlyContinue) {
    throw 'Close Windhawk before preparing the portable instance.'
}

$installer = Join-Path $output 'windhawk-2.0.0-alpha.8-setup.exe'
$url = 'https://github.com/ramensoftware/windhawk/releases/download/2.0.0-alpha.8/windhawk_setup.exe'
$expected = '7327471E2D93A8C58481321CCE546632ADB146CF125D17D7144D0E7D2A064967'
if (!(Test-Path -LiteralPath $installer)) {
    Invoke-WebRequest -Uri $url -OutFile $installer
}
if ((Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash -ne $expected -or
    (Get-AuthenticodeSignature -LiteralPath $installer).Status -ne 'Valid') {
    throw 'Windhawk setup verification failed.'
}

$setup = Start-Process -FilePath $installer -ArgumentList ('/portable', '/S', "/D=$portable") -PassThru -WindowStyle Hidden
if (!$setup.WaitForExit(120000) -or $setup.ExitCode -ne 0) {
    throw 'Windhawk portable setup did not finish successfully.'
}

$cli = Join-Path $portable 'windhawk-cli.exe'
try {
    $settings = @(
        @('engine.exclude', '*'),
        @('engine.include', (Join-Path $env:SystemRoot 'explorer.exe')),
        @('hideTrayIcon', 'true'),
        @('disableUpdateCheck', 'true')
    )
    foreach ($setting in $settings) {
        & $cli --app-root $portable app settings set --confirm-app-restart @setting
        if ($LASTEXITCODE -ne 0) { throw "Could not set $($setting[0])." }
    }
    & $cli --app-root $portable mod install windows-11-taskbar-styler 1.11.2 --disabled
    if ($LASTEXITCODE -ne 0) { throw 'Could not install Windows 11 Taskbar Styler.' }
}
finally {
    $engine = Get-Process -Name windhawk -ErrorAction SilentlyContinue |
        Where-Object Path -EQ (Join-Path $portable 'windhawk.exe')
    if ($engine) {
        $stop = Start-Process -FilePath $engine.Path -ArgumentList '-exit -wait' -PassThru -WindowStyle Hidden
        if (!$stop.WaitForExit(30000)) { throw 'Windhawk did not finish closing.' }
    }
}
