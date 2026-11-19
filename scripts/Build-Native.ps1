param(
    [ValidateSet('x86_64-pc-windows-msvc')][string]$Target,
    [string]$DestinationExecutable
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Join-Path $root 'src\rust\Cargo.toml'
$rootExe = Join-Path $root 'Folio.exe'
$publishingRoot = -not $DestinationExecutable
$destination = $rootExe
if ($DestinationExecutable) {
    $destination = [IO.Path]::GetFullPath($DestinationExecutable)
}
$targetDir = Join-Path $root 'work\rust-build'
$targetArgs = @()
$buildDir = $targetDir
if ($Target) {
    $targetArgs = @('--target', $Target)
    $buildDir = Join-Path $targetDir $Target
}
$rustRoot = Join-Path $root 'work\tools\rust-1.100.0-gnu'
$localCargo = Join-Path $rustRoot 'bin\cargo.exe'
$binutils = Join-Path $root 'work\tools\mingw64-binutils\mingw64\bin'

function Assert-RootExecutableAvailable {
    foreach ($process in @(Get-CimInstance -ClassName Win32_Process -Filter "Name = 'Folio.exe'")) {
        if ([string]::IsNullOrWhiteSpace($process.ExecutablePath) -or
            [string]::Equals($process.ExecutablePath, $rootExe, [StringComparison]::OrdinalIgnoreCase)) {
            throw ('Close the running Folio window before replacing its executable. PID ' + $process.ProcessId)
        }
    }
}

$stored = @{}
foreach ($name in @('Path','CARGO_HOME','CARGO_TARGET_DIR','FOLIO_WINDRES')) {
    $stored[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    if ((Test-Path -LiteralPath $localCargo -PathType Leaf) -and -not $Target) {
        $cargo = $localCargo
        $rustBin = Join-Path $rustRoot 'bin'
        $rustSupportBin = Join-Path $rustRoot 'lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained'
        $windres = Join-Path $binutils 'windres.exe'
        if (-not (Test-Path -LiteralPath $windres -PathType Leaf)) { throw 'The project-local GNU Windres executable is missing.' }
        $env:Path = $rustBin + ';' + $binutils + ';' + $rustSupportBin + ';' + $env:Path
        $env:FOLIO_WINDRES = $windres
    } else {
        $cargo = (Get-Command cargo.exe -ErrorAction Stop).Source
    }

    $env:CARGO_HOME = Join-Path $root 'work\cargo-home'
    $env:CARGO_TARGET_DIR = $targetDir
    & $cargo build --manifest-path $manifest --release --locked @targetArgs
    if ($LASTEXITCODE -ne 0) { throw 'Folio Rust release build failed.' }

    $candidate = Join-Path $buildDir 'release\Folio.exe'

    if ($publishingRoot) { Assert-RootExecutableAvailable }
    Copy-Item -LiteralPath $candidate -Destination $destination -Force
    $expected = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash
    $actual = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
    if ($expected -ne $actual) { throw 'Folio delivery hash mismatch.' }
    Write-Output ('Folio.exe SHA-256 ' + $actual)
} finally {
    foreach ($name in $stored.Keys) {
        [Environment]::SetEnvironmentVariable($name, $stored[$name], 'Process')
    }
}
