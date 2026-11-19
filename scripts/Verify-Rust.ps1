param([string]$ExecutablePath = (Join-Path $PSScriptRoot '..\Folio.exe'))

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$exe = (Resolve-Path -LiteralPath $ExecutablePath).Path
$harness = Join-Path $root 'work\rust-build\release\FolioVerify.exe'
if (-not (Test-Path -LiteralPath $harness -PathType Leaf)) {
    throw 'FolioVerify.exe is missing. Build the Rust release before running verification.'
}
$previous = [Console]::OutputEncoding
try {
    [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
    & $harness $exe $root
    exit $LASTEXITCODE
} finally {
    [Console]::OutputEncoding = $previous
}
