param([string]$Package)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Join-Path $root 'src\rust\Cargo.toml'
$versionLine = Select-String -LiteralPath $manifest -Pattern '^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$' | Select-Object -First 1
if ($null -eq $versionLine) { throw 'Missing Folio version.' }
$version = $versionLine.Matches[0].Groups[1].Value
if (-not $Package) { $Package = Join-Path $root ('generated\Folio-' + $version + '-win-x64.msi') }
$msi = (Resolve-Path -LiteralPath $Package).ProviderPath

$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.GetType().InvokeMember('OpenDatabase','InvokeMethod',$null,$installer,@($msi,0))
$summary = $database.GetType().InvokeMember('SummaryInformation','GetProperty',$null,$database,@(0))
$template = $summary.GetType().InvokeMember('Property','GetProperty',$null,$summary,@(7))
if ($template -notlike 'x64;*') { throw ('Unexpected MSI architecture ' + $template) }
$query = 'SELECT `Value` FROM `Property` WHERE `Property` = ' + [char]39 + 'ProductVersion' + [char]39
$view = $database.GetType().InvokeMember('OpenView','InvokeMethod',$null,$database,@($query))
$null = $view.GetType().InvokeMember('Execute','InvokeMethod',$null,$view,$null)
$row = $view.GetType().InvokeMember('Fetch','InvokeMethod',$null,$view,$null)
if ($null -eq $row -or $row.GetType().InvokeMember('StringData','GetProperty',$null,$row,@(1)) -ne $version) {
    throw 'The MSI version differs from Cargo.toml.'
}
$null = $view.GetType().InvokeMember('Close','InvokeMethod',$null,$view,$null)

$test = Join-Path $root ('work\installer-test\' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $test
$image = Join-Path $test 'image'
$log = Join-Path $test 'msi.log'
$argsForMsi = @('/a',('"' + $msi + '"'),'/qn',('TARGETDIR="' + $image + '"'),'/l*v',('"' + $log + '"'))
$extractor = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\msiexec.exe') -ArgumentList $argsForMsi -WindowStyle Hidden -Wait -PassThru
if ($extractor.ExitCode -ne 0) { throw ('MSI administrative extraction failed. See ' + $log) }
$programs = @(Get-ChildItem -LiteralPath $image -Recurse -File -Filter Folio.exe)
if ($programs.Count -ne 1) { throw 'Expected exactly one Folio.exe in the MSI image.' }
$program = $programs[0]
if (-not (Test-Path -LiteralPath (Join-Path $program.DirectoryName 'LICENSE') -PathType Leaf)) {
    throw 'Application license is missing from the MSI image.'
}

$previousAppData = $env:APPDATA
$process = $null
try {
    $env:APPDATA = Join-Path $test 'profile'
    $process = Start-Process -FilePath $program.FullName -WorkingDirectory $program.DirectoryName -WindowStyle Hidden -PassThru
    if (-not $process.WaitForInputIdle(20000) -or $process.HasExited) {
        throw 'The packaged Folio executable did not start its Windows message loop.'
    }
    $state = Join-Path $env:APPDATA 'Folio\native\reader-library.json'
    if (-not (Test-Path -LiteralPath $state -PathType Leaf)) {
        throw 'The packaged Folio executable did not initialize the isolated profile.'
    }
    Write-Output ('Verified MSI version ' + $version + ', x64 payload and isolated application startup.')
} finally {
    $env:APPDATA = $previousAppData
    if ($null -ne $process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
    }
}
