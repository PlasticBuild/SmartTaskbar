param([Parameter(Mandatory)][string]$OutputDirectory, [switch]$CreateShortcuts)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
$picker = Join-Path $output 'SmartTaskbar.Appearance.exe'
if ($CreateShortcuts -and !(Test-Path -LiteralPath $picker -PathType Leaf)) {
    throw 'Publish SmartTaskbar Appearance into the output directory before creating its shortcuts.'
}
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
    $systemApps = Join-Path $env:SystemRoot 'SystemApps'
    $shellIncludes = @((Join-Path $env:SystemRoot 'explorer.exe'), (Join-Path ([Environment]::SystemDirectory) 'ShellHost.exe'))
    $shellNames = @('StartMenuExperienceHost.exe', 'SearchHost.exe', 'SearchApp.exe', 'ShellExperienceHost.exe')
    foreach ($directory in (Get-ChildItem -LiteralPath $systemApps -Directory)) {
        foreach ($name in $shellNames) {
            $hostPath = Join-Path $directory.FullName $name
            if (Test-Path -LiteralPath $hostPath -PathType Leaf) { $shellIncludes += $hostPath }
        }
    }
    $settings = @(
        @('engine.exclude', '*'),
        @('engine.include', ($shellIncludes -join ',')),
        @('hideTrayIcon', 'true'),
        @('disableUpdateCheck', 'true')
    )
    foreach ($setting in $settings) {
        & $cli --app-root $portable app settings set --confirm-app-restart @setting
        if ($LASTEXITCODE -ne 0) { throw "Could not set $($setting[0])." }
    }
    $modules = @(
        @('windows-11-taskbar-styler', '1.11.2'),
        @('windows-11-start-menu-styler', '1.8'),
        @('windows-11-notification-center-styler', '1.8'),
        @('shell-flyout-positions', '1.3.1')
    )
    foreach ($module in $modules) {
        & $cli --app-root $portable mod install @module --disabled
        if ($LASTEXITCODE -ne 0) { throw "Could not install $($module[0])." }
    }
}
finally {
    $engine = Get-Process -Name windhawk -ErrorAction SilentlyContinue |
        Where-Object Path -EQ (Join-Path $portable 'windhawk.exe')
    if ($engine) {
        $stop = Start-Process -FilePath $engine.Path -ArgumentList '-exit -wait' -PassThru -WindowStyle Hidden
        if (!$stop.WaitForExit(30000)) { throw 'Windhawk did not finish closing.' }
    }
}

if ($CreateShortcuts) {
    $shell = New-Object -ComObject WScript.Shell
    foreach ($folder in @('DesktopDirectory', 'Programs')) {
        $link = $shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath($folder)) 'SmartTaskbar Appearance.lnk'))
        $link.TargetPath = $picker
        $link.WorkingDirectory = $output
        $link.Arguments = ''
        $link.IconLocation = "$picker,0"
        $link.Description = 'Customize the Windows taskbar'
        $link.Save()
    }
}
