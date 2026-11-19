param(
    [switch]$RestoreTools,
    [switch]$Verify
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Join-Path $root 'src\rust\Cargo.toml'
$versionLine = Select-String -LiteralPath $manifest -Pattern '^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$' | Select-Object -First 1
if ($null -eq $versionLine) { throw 'The Cargo package must have a three-part numeric version.' }
$version = $versionLine.Matches[0].Groups[1].Value

$wix = Join-Path $root 'work\tools\wix-5.0.2\wix.exe'
$build = Join-Path $root 'work\installer-build'
$extension = Join-Path $build '.wix\extensions\WixToolset.UI.wixext\5.0.2\wixext5\WixToolset.UI.wixext.dll'
$executable = Join-Path $root 'Folio.exe'
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw 'Build Folio.exe before packaging.' }

$generated = Join-Path $root 'generated'
New-Item -ItemType Directory -Path $generated,$build -Force | Out-Null
$name = 'Folio-' + $version + '-win-x64.msi'
$candidate = Join-Path $build $name
$destination = Join-Path $generated $name
$previousDotnetRoot = $env:DOTNET_ROOT
try {
    $localDotnet = Join-Path $root 'work\tools\dotnet'
    $dotnet = Join-Path $localDotnet 'dotnet.exe'
    if (Test-Path -LiteralPath $dotnet -PathType Leaf) {
        $env:DOTNET_ROOT = $localDotnet
    } elseif ($RestoreTools) {
        $dotnet = (Get-Command dotnet.exe -ErrorAction Stop).Source
    }
    if ($RestoreTools -and -not (Test-Path -LiteralPath $wix -PathType Leaf)) {
        & $dotnet tool install wix --version 5.0.2 --tool-path (Split-Path -Parent $wix) --allow-roll-forward `
            --add-source https://api.nuget.org/v3/index.json
        if ($LASTEXITCODE -ne 0) { throw 'Could not restore the WiX tool.' }
    }
    if ($RestoreTools -and -not (Test-Path -LiteralPath $extension -PathType Leaf)) {
        Push-Location -LiteralPath $build
        try {
            & $wix extension add WixToolset.UI.wixext/5.0.2
            if ($LASTEXITCODE -ne 0) { throw 'Could not restore the WiX UI extension.' }
        } finally {
            Pop-Location
        }
    }
    foreach ($required in @($wix, $extension)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw ('Run with -RestoreTools to obtain: ' + $required) }
    }
    & $wix build (Join-Path $root 'src\installer\Folio.wxs') -arch x64 -culture zh-CN `
        -ext $extension -d ('Version=' + $version) -d ('ProjectRoot=' + $root) `
        -intermediatefolder (Join-Path $build 'obj') -out $candidate
    if ($LASTEXITCODE -ne 0) { throw 'WiX could not build the Folio installer.' }
} finally {
    $env:DOTNET_ROOT = $previousDotnetRoot
}

if ($Verify) {
    $verification = Join-Path $build ('verify-' + [Guid]::NewGuid().ToString('N'))
    $image = Join-Path $verification 'admin-image'
    New-Item -ItemType Directory -Path $image | Out-Null
    $log = Join-Path $verification 'extract.log'
    $arguments = @('/a', ('"' + $candidate + '"'), '/qn', ('TARGETDIR="' + $image + '"'), '/l*v', ('"' + $log + '"'))
    $installer = Start-Process -FilePath (Get-Command msiexec.exe).Source -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    if ($installer.ExitCode -ne 0) { throw ('Windows Installer could not extract the package. See ' + $log) }
    $payload = @(Get-ChildItem -LiteralPath $image -Filter 'Folio.exe' -File -Recurse)
    $exeHash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
    if ($payload.Count -ne 1 -or (Get-FileHash -LiteralPath $payload[0].FullName -Algorithm SHA256).Hash -ne $exeHash) {
        throw 'The extracted MSI executable does not match Folio.exe.'
    }
    Write-Output ('Verified MSI payload SHA-256 ' + $exeHash)
}
Copy-Item -LiteralPath $candidate -Destination $destination -Force
$expected = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash
$actual = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
if ($actual -ne $expected) { throw 'Installer delivery hash mismatch.' }
Write-Output $destination
Write-Output ('SHA-256 ' + $actual)
