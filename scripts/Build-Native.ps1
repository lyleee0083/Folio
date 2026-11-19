param(
    [switch]$Test,
    [switch]$UnitTest,
    [switch]$KeepArtifacts
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Join-Path $root 'src\rust\Cargo.toml'
$rootExe = Join-Path $root 'Folio.exe'
$targetDir = Join-Path $root 'work\rust-build'
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
    if (Test-Path -LiteralPath $localCargo -PathType Leaf) {
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
    if ($Test -or $UnitTest) {
        & $cargo test --manifest-path $manifest --locked
        if ($LASTEXITCODE -ne 0) { throw 'Folio Rust tests failed.' }
    }
    & $cargo build --manifest-path $manifest --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Folio Rust release build failed.' }

    $candidate = Join-Path $targetDir 'release\Folio.exe'
    if ($Test) {
        $powershell = (Get-Command powershell.exe -ErrorAction Stop).Source
        & $powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'Verify-Rust.ps1') -ExecutablePath $candidate
        if ($LASTEXITCODE -ne 0) { throw 'Folio native window verification failed.' }
    }

    Assert-RootExecutableAvailable
    Copy-Item -LiteralPath $candidate -Destination $rootExe -Force
    $expected = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash
    $actual = (Get-FileHash -LiteralPath $rootExe -Algorithm SHA256).Hash
    if ($expected -ne $actual) { throw 'Folio delivery hash mismatch.' }
    Write-Output ('Folio.exe SHA-256 ' + $actual)
    if ($KeepArtifacts) { Write-Output ('Rust build artifacts are in ' + $targetDir) }
} finally {
    foreach ($name in $stored.Keys) {
        [Environment]::SetEnvironmentVariable($name, $stored[$name], 'Process')
    }
}
