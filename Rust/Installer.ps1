param(
    [Parameter(Mandatory)][string]$StyleRoot,
    [string]$OutputDirectory,
    [string]$Compiler
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (!$OutputDirectory) { $OutputDirectory = Join-Path $repo 'bin/installer' }
if (!$Compiler) { $Compiler = Join-Path $repo 'bin/toolchain/nsis/nsis-3.11/makensis.exe' }
$compilerPath = (Resolve-Path -LiteralPath $Compiler).Path
$style = (Resolve-Path -LiteralPath $StyleRoot).Path
$build = Join-Path $repo ('bin/installer-payload/' + [Guid]::NewGuid().ToString('N'))
$cache = Join-Path $repo 'bin/toolchain/windhawk-source'
$null = New-Item -ItemType Directory -Force -Path $build,$OutputDirectory,$cache
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
$app = Join-Path $build 'App'
& (Join-Path $PSScriptRoot 'Stage.ps1') -OutputDirectory $app

$hostRoot = Join-Path $build 'Windhawk'
$modRoot = Join-Path $hostRoot 'AppData/Engine/Mods'
$sourceRoot = Join-Path $hostRoot 'AppData/ModsSource'
$null = New-Item -ItemType Directory -Force -Path $modRoot,$sourceRoot
foreach ($item in 'Engine','ModsRuntime','windhawk.exe','windhawk-cli.exe','windhawk-core.dll',
    'windhawk-mod.exe','windhawk-mod-elevated.exe','windhawk-mod-uiaccess.exe','windhawk-ui.exe','windhawk.ini') {
    Copy-Item -LiteralPath (Join-Path $style $item) -Destination $hostRoot -Recurse
}
if (!(Test-Path -LiteralPath (Join-Path $hostRoot 'Engine/2.0.0') -PathType Container)) {
    throw 'Expected the portable Windhawk 2.0.0-alpha.8 engine.'
}
$appSettings = @'
[Settings]
Language=en
LoggingVerbosity=0
HideTrayIcon=1
DisableUpdateCheck=1
SafeMode=0
'@
Set-Content -LiteralPath (Join-Path $hostRoot 'AppData/settings.ini') -Value $appSettings -Encoding ascii
foreach ($arch in '32','64','arm64') {
    $libs = Join-Path $modRoot $arch
    $null = New-Item -ItemType Directory -Path $libs
    foreach ($file in 'libc++.whl','libunwind.whl','windhawk-mod-shim.dll') {
        Copy-Item -LiteralPath (Join-Path $style "AppData/Engine/Mods/$arch/$file") -Destination $libs
    }
}
$mods = [ordered]@{
    'windows-11-taskbar-styler' = '1.11.2'
    'windows-11-start-menu-styler' = '1.8'
    'windows-11-notification-center-styler' = '1.8'
    'shell-flyout-positions' = '1.3.1'
}
foreach ($id in $mods.Keys) {
    $ini = Get-Content -LiteralPath (Join-Path $style "AppData/Engine/Mods/$id.ini") -Raw
    $header = ($ini -split '(?m)^\[Settings\]\s*$', 2)[0]
    $version = [regex]::Escape($mods[$id])
    $library = [regex]::Match($header, ('(?m)^LibraryFileName=(' + [regex]::Escape($id) + "_${version}_\d+\.dll)\r?$"))
    if ($header -notmatch "(?m)^Version=$version\r?$" -or !$library.Success) {
        throw "Expected $id $($mods[$id])."
    }
    $clean = $header -replace '(?m)^Disabled=\d+','Disabled=1' -replace '(?m)^SettingsChangeTime=\d+','SettingsChangeTime=0'
    Set-Content -LiteralPath (Join-Path $modRoot "$id.ini") -Value ($clean.TrimEnd() + "`r`n[Settings]") -Encoding ascii
    Copy-Item -LiteralPath (Join-Path $style "AppData/Engine/Mods/64/$($library.Groups[1].Value)") -Destination (Join-Path $modRoot '64')
    Copy-Item -LiteralPath (Join-Path $style "AppData/ModsSource/$id.wh.cpp") -Destination $sourceRoot
}

$sources = @(
    @{
        Name = 'windhawk-2.0.0-alpha.8.tar.gz'
        Url = 'https://codeload.github.com/ramensoftware/windhawk/tar.gz/refs/tags/2.0.0-alpha.8'
        Hash = '25DEA44C86E44078CBF9B44BFD6D56B480B1CCC39E8A34BA8315BB807D09249C'
    },
    @{
        Name = 'Windhawk-GPL-3.0.txt'
        Url = 'https://raw.githubusercontent.com/ramensoftware/windhawk/refs/tags/2.0.0-alpha.8/LICENSE'
        Hash = 'CA24EBC0D4FA9C155C6F9459DD1DB7D762C09C0200B3793D00B5D09E60ABF431'
    }
)
foreach ($asset in $sources) {
    $file = Join-Path $cache $asset.Name
    if (!(Test-Path -LiteralPath $file)) {
        Invoke-WebRequest -Uri $asset.Url -OutFile $file
    }
    if ((Get-FileHash -LiteralPath $file).Hash -ne $asset.Hash) {
        throw "Could not verify $($asset.Name)."
    }
    Copy-Item -LiteralPath $file -Destination (Join-Path $app 'licenses')
}

$icon = Join-Path $repo 'Sources/SmartTaskbar.Win10/Resources/logo_blue_128.ico'
$installer = Join-Path $output 'SmartTaskbar-Setup.exe'
& $compilerPath '/V3' "/DPAYLOAD=$build" "/DOUTPUT=$installer" "/DICON=$icon" (Join-Path $PSScriptRoot 'SmartTaskbar.nsi')
if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $installer)) { throw 'Could not build SmartTaskbar Setup.' }
Get-Item -LiteralPath $installer
