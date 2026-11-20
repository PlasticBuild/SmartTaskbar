param([ValidateSet('test','build','check','fetch','fmt','clippy','metadata')][string]$Action='test')
$ErrorActionPreference='Stop'
$project=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tools=Join-Path $project 'bin/toolchain/rust'
$localCargo=Join-Path $tools 'cargo/bin/cargo.exe'
$cargo=if (Test-Path -LiteralPath $localCargo) {
    $env:RUSTUP_HOME=Join-Path $tools 'rustup'
    $localCargo
} else { (Get-Command cargo -ErrorAction Stop).Source }
$env:CARGO_HOME=Join-Path $tools 'cargo'
$env:CARGO_TARGET_DIR=Join-Path $project 'bin/rust-target'
if (!$env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS='2' }
$env:TEMP=Join-Path $tools 'tmp'
$env:TMP=$env:TEMP
$null=New-Item -Path $env:TEMP -ItemType Directory -Force
$env:PATH="$(Split-Path -Parent $cargo);$env:PATH"
Push-Location $PSScriptRoot
try {
    $argsList=@($Action,'--locked')
    if ($Action -eq 'fmt') { $argsList=@('fmt','--all') }
    if ($Action -eq 'build') { $argsList+='--release' }
    if ($Action -eq 'clippy') { $argsList+=@('--all-targets','--','-D','warnings') }
    & $cargo @argsList
    if ($LASTEXITCODE) { throw "Cargo $Action failed ($LASTEXITCODE)." }
} finally { Pop-Location }
