param([string]$Package, [string]$PreviousPackage)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifest = Join-Path $root 'src\rust\Cargo.toml'
$versionLine = Select-String -LiteralPath $manifest -Pattern '^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$' | Select-Object -First 1
if ($null -eq $versionLine) { throw 'Missing Folio version.' }
$version = $versionLine.Matches[0].Groups[1].Value
if (-not $Package) { $Package = Join-Path $root ('generated\Folio-win-x64-' + $version + '.msi') }
$msi = (Resolve-Path -LiteralPath $Package).ProviderPath

$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.GetType().InvokeMember('OpenDatabase','InvokeMethod',$null,$installer,@($msi,0))
$summary = $database.GetType().InvokeMember('SummaryInformation','GetProperty',$null,$database,@(0))
$template = $summary.GetType().InvokeMember('Property','GetProperty',$null,$summary,@(7))
if ($template -notlike 'x64;*') { throw ('Unexpected MSI architecture ' + $template) }
function Get-MsiProperty($Path, $Name) {
    $db = $installer.GetType().InvokeMember('OpenDatabase','InvokeMethod',$null,$installer,@($Path,0))
    $quote = [char]96
    $literal = [char]39
    $query = "SELECT ${quote}Value${quote} FROM ${quote}Property${quote} WHERE ${quote}Property${quote} = $literal$Name$literal"
    $view = $db.GetType().InvokeMember('OpenView','InvokeMethod',$null,$db,@($query))
    try {
        $null = $view.GetType().InvokeMember('Execute','InvokeMethod',$null,$view,$null)
        $row = $view.GetType().InvokeMember('Fetch','InvokeMethod',$null,$view,$null)
        if ($null -eq $row) { throw "Missing MSI property $Name" }
        $row.GetType().InvokeMember('StringData','GetProperty',$null,$row,@(1))
    } finally {
        $null = $view.GetType().InvokeMember('Close','InvokeMethod',$null,$view,$null)
    }
}

if ((Get-MsiProperty $msi 'ProductVersion') -ne $version) {
    throw 'The MSI version differs from Cargo.toml.'
}

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
$programVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo($program.FullName)
if ($programVersion.FileVersion -cne $version -or $programVersion.ProductVersion -cne $version) {
    throw "The Folio.exe file and product versions must both match the MSI version $version."
}
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
    Write-Output ('Verified MSI and Folio.exe version ' + $version + ', x64 payload and isolated application startup.')
} finally {
    $env:APPDATA = $previousAppData
    if ($null -ne $process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
    }
}

if ($PreviousPackage) {
    $previous = (Resolve-Path -LiteralPath $PreviousPackage).ProviderPath
    $oldVersion = Get-MsiProperty $previous 'ProductVersion'
    $oldCode = Get-MsiProperty $previous 'ProductCode'
    $newCode = Get-MsiProperty $msi 'ProductCode'
    if ([version]$oldVersion -ge [version]$version -or $oldCode -eq $newCode -or
        (Get-MsiProperty $previous 'UpgradeCode') -ne (Get-MsiProperty $msi 'UpgradeCode')) {
        throw 'The upgrade baseline must be an older version of the same MSI product family.'
    }

    $installFolder = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Folio'
    $shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Folio.lnk'
    $registry = 'HKCU:\Software\Folio\Installer'

    function Get-ProductState($Code) {
        $installer.GetType().InvokeMember('ProductState','GetProperty',$null,$installer,@($Code))
    }
    function Invoke-LifecycleStep($Operation, $Target, $Name) {
        $stepLog = Join-Path $test ($Name + '.log')
        $args = @($Operation, ('"' + $Target + '"'), '/qn', '/norestart', '/l*v', ('"' + $stepLog + '"'))
        $stepProcess = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\msiexec.exe') -ArgumentList $args -WindowStyle Hidden -Wait -PassThru
        if ($stepProcess.ExitCode -ne 0) {
            throw "MSI $Name failed with exit code $($stepProcess.ExitCode). See $stepLog"
        }
    }
    function Test-InstallerMarker {
        if (-not (Test-Path -LiteralPath $registry)) { return $false }
        $key = Get-Item -LiteralPath $registry
        $null -ne $key.GetValue('Installed') -or $null -ne $key.GetValue('Notices')
    }
    function Assert-Installed($Code) {
        if ((Get-ProductState $Code) -ne 5) { throw "MSI product $Code is not installed." }
        $exe = Join-Path $installFolder 'Folio.exe'
        if (-not (Test-Path -LiteralPath $exe -PathType Leaf) -or
            -not (Test-Path -LiteralPath $shortcut -PathType Leaf)) {
            throw 'The installed executable or Start menu shortcut is missing.'
        }
        $shell = New-Object -ComObject WScript.Shell
        if ($shell.CreateShortcut($shortcut).TargetPath -ne $exe) {
            throw 'The Start menu shortcut targets the wrong executable.'
        }
        if ((Get-ItemProperty -LiteralPath $registry -Name Installed).Installed -ne 1) {
            throw 'The per-user installer registry marker is missing.'
        }
    }

    if ((Get-ProductState $oldCode) -ne -1 -or (Get-ProductState $newCode) -ne -1 -or
        (Test-Path -LiteralPath $installFolder) -or (Test-Path -LiteralPath $shortcut) -or
        (Test-InstallerMarker)) {
        throw 'The MSI lifecycle test needs a clean Windows profile.'
    }

    try {
        Invoke-LifecycleStep '/i' $previous 'install-baseline'
        Assert-Installed $oldCode
        Invoke-LifecycleStep '/i' $msi 'upgrade'
        Assert-Installed $newCode
        if ((Get-ProductState $oldCode) -ne -1) { throw 'The old product remained after upgrading.' }
        if ((Get-FileHash -LiteralPath (Join-Path $installFolder 'Folio.exe')).Hash -ne
            (Get-FileHash -LiteralPath $program.FullName).Hash) {
            throw 'The upgraded executable differs from the validated MSI payload.'
        }
        Invoke-LifecycleStep '/x' $newCode 'uninstall'
        if ((Get-ProductState $newCode) -ne -1 -or (Test-Path -LiteralPath $installFolder) -or
            (Test-Path -LiteralPath $shortcut) -or (Test-InstallerMarker)) {
            throw 'The MSI left installed components or registration after uninstall.'
        }
        Write-Output "Verified per-user installation, upgrade from $oldVersion to $version, and uninstall."
    } finally {
        foreach ($code in @($newCode,$oldCode)) {
            if ((Get-ProductState $code) -eq 5) { Invoke-LifecycleStep '/x' $code ('cleanup-' + $code) }
        }
    }
}
