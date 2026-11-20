param([string]$OutputDirectory, [string]$StyleRoot, [switch]$CreateShortcuts)
$ErrorActionPreference='Stop'
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (!$OutputDirectory) { $OutputDirectory=Join-Path $repo 'bin/rust-preview' }
$out=[IO.Path]::GetFullPath($OutputDirectory)
$null=New-Item -Path $out -ItemType Directory -Force
& (Join-Path $PSScriptRoot 'Build.ps1') build
$cargo=Join-Path $repo 'bin/toolchain/rust/cargo/bin/cargo.exe'
$rustc=Join-Path $repo 'bin/toolchain/rust/cargo/bin/rustc.exe'
if (!(Test-Path -LiteralPath $cargo)){$cargo=(Get-Command cargo).Source;$rustc=(Get-Command rustc).Source}
$metadata=& $cargo metadata --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') --locked --offline --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE) {throw 'Cannot read crate license metadata.'}
$legal=Join-Path $out 'licenses'
$null=New-Item -ItemType Directory -Force -Path $legal
$notices=foreach($crate in $metadata.packages){
    if ($crate.name -eq 'smarttaskbar-rs') {continue}
    $source=Split-Path -Parent $crate.manifest_path
    $name="$($crate.name)-$($crate.version)"
    $folder=Join-Path $legal $name
    $null=New-Item -ItemType Directory -Force -Path $folder
    $files=@(Get-ChildItem -LiteralPath $source -File | Where-Object Name -Match '^(licen[cs]e|copying|notice|copyright)')
    if (!$files) {throw "No license text located for $name"}
    foreach ($file in $files){Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $folder $file.Name) -Force}
    "$name  $($crate.license)"
}
$notices | Set-Content -LiteralPath (Join-Path $legal 'crates.txt') -Encoding utf8
$sysroot=& $rustc --print sysroot
$stdLicense=Join-Path $sysroot 'share/doc/rust/COPYRIGHT-library.html'
Copy-Item -LiteralPath $stdLicense -Destination (Join-Path $legal 'Rust-standard-library.html') -Force
Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination (Join-Path $out 'LICENSE-SmartTaskbar.txt') -Force
foreach($name in 'README.md','NOTICES.md'){Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $out $name) -Force}
if($StyleRoot){
    $style=(Resolve-Path -LiteralPath $StyleRoot).Path
    if(!(Test-Path -LiteralPath (Join-Path $style 'windhawk-cli.exe'))){throw 'Expected a prepared portable Windhawk directory.'}
    [IO.File]::WriteAllText((Join-Path $out 'style-root.txt'),$style)
} elseif(Test-Path -LiteralPath (Join-Path $out 'style-root.txt') -PathType Leaf) {
    Remove-Item -LiteralPath (Join-Path $out 'style-root.txt')
}
$compiled=Join-Path $metadata.target_directory 'release/smarttaskbar-rs.exe'
$binary=Join-Path $out 'smarttaskbar-rs.exe'
Copy-Item -LiteralPath $compiled -Destination $binary -Force
if((Get-FileHash -LiteralPath $binary).Hash -ne (Get-FileHash -LiteralPath $compiled).Hash){throw 'The staged executable failed verification.'}
if ($CreateShortcuts) {
    $shell=New-Object -ComObject WScript.Shell
    foreach($folder in 'DesktopDirectory','Programs'){
        $shortcut=$shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath($folder)) 'SmartTaskbar Rust.lnk'))
        $shortcut.TargetPath=$binary
        $shortcut.Arguments='--settings'
        $shortcut.WorkingDirectory=$out
        $shortcut.Save()
    }
}
"Staged $binary"
