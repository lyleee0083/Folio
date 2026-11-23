param(
    [Parameter(Mandatory = $true)]
    [string]$DestinationExecutable
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Join-Path $root 'src\rust\Cargo.toml'
$target = 'x86_64-pc-windows-msvc'
$targetDir = Join-Path $root 'work\rust-build'
$destination = [IO.Path]::GetFullPath($DestinationExecutable)
$cargo = (Get-Command cargo.exe -ErrorAction Stop).Source
$previousCargoHome = $env:CARGO_HOME
$previousTargetDir = $env:CARGO_TARGET_DIR

try {
    $env:CARGO_HOME = Join-Path $root 'work\cargo-home'
    $env:CARGO_TARGET_DIR = $targetDir
    & $cargo build --manifest-path $manifest --release --locked --target $target
    $buildExit = $LASTEXITCODE
    if ($buildExit -ne 0) { throw ('Folio Rust release build failed with exit code ' + $buildExit + '.') }

    $candidate = Join-Path $targetDir "$target\release\Folio.exe"
    Copy-Item -LiteralPath $candidate -Destination $destination -Force
    $expected = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash
    $actual = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
    if ($expected -ne $actual) { throw 'Folio delivery hash mismatch.' }
    Write-Output ('Folio.exe SHA-256 ' + $actual)
} finally {
    $env:CARGO_HOME = $previousCargoHome
    $env:CARGO_TARGET_DIR = $previousTargetDir
}
