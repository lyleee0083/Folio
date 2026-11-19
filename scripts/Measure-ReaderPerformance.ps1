param(
    [string]$ExecutablePath = (Join-Path (Split-Path -Parent $PSScriptRoot) 'Folio.exe'),
    [ValidateRange(12,20)][int]$Iterations = 16,
    [ValidateRange(60,240)][int]$TimeoutSeconds = 210,
    [switch]$RapidSwitchProbe
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$outputRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'outputs\2026-11-18-rust-rewrite\performance'))
$fixtureRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'work\2026-10-24-reader-performance\fixtures'))
$manifestPath = Join-Path $fixtureRoot 'manifest.json'
$executable = [IO.Path]::GetFullPath($ExecutablePath)

if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw ('Executable not found: ' + $executable) }
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw 'Fixture manifest is missing.' }

$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
foreach ($fixtureName in @('typical-chinese-prose.md','large-gfm.md')) {
    $fixtureEntry = @($manifest | Where-Object { $_.Name -eq $fixtureName }) | Select-Object -First 1
    if ($null -eq $fixtureEntry) { throw ('Fixture missing from manifest: ' + $fixtureName) }
    $fixturePath = Join-Path $fixtureRoot $fixtureName
    if (-not (Test-Path -LiteralPath $fixturePath -PathType Leaf)) { throw ('Fixture file is missing: ' + $fixturePath) }
    $fixtureFile = Get-Item -LiteralPath $fixturePath -Force
    $fixtureHash = (Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash
    if ($fixtureFile.Length -ne [long]$fixtureEntry.LengthBytes -or $fixtureHash -ne [string]$fixtureEntry.SHA256) {
        throw ('Fixture differs from its frozen manifest: ' + $fixtureName)
    }
}

$executableName = [IO.Path]::GetFileName($executable).Replace("'", "''")
$processFilter = "Name = 'Folio.exe' OR Name = '$executableName'"
$processes = @(Get-CimInstance -ClassName Win32_Process -Filter $processFilter -ErrorAction Stop)
if ($processes.Count -gt 0) {
    $paths = @($processes | ForEach-Object { $_.ExecutablePath } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    throw ('Close existing Folio windows before a performance run. Running process paths: ' + ($paths -join '; '))
}

$ancestor = $outputRoot
while (-not [string]::IsNullOrWhiteSpace($ancestor)) {
    if (Test-Path -LiteralPath $ancestor) {
        $item = Get-Item -LiteralPath $ancestor -Force -ErrorAction Stop
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw ('Performance output cannot traverse a reparse point: ' + $ancestor)
        }
    }
    $parent = [IO.Path]::GetDirectoryName($ancestor.TrimEnd('\'))
    if ([string]::Equals($parent,$ancestor,[StringComparison]::OrdinalIgnoreCase)) { break }
    $ancestor = $parent
}

New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
$runId = (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmssfff') + '-' + $PID.ToString() + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8)
$runDirectory = [IO.Path]::GetFullPath((Join-Path $outputRoot $runId))
$outputPrefix = $outputRoot.TrimEnd('\') + '\'
if (-not $runDirectory.StartsWith($outputPrefix,[StringComparison]::OrdinalIgnoreCase)) { throw 'Run output escaped the task output directory.' }
New-Item -ItemType Directory -Path $runDirectory -ErrorAction Stop | Out-Null
$profilePath = Join-Path $runDirectory 'profile'
if (Test-Path -LiteralPath $profilePath) { throw 'The unique run profile path already exists.' }

$previousProfile = [Environment]::GetEnvironmentVariable('FOLIO_TEST_USER_DATA','Process')
$process = $null
try {
    [Environment]::SetEnvironmentVariable('FOLIO_TEST_USER_DATA',$profilePath,'Process')
    $argumentString = '--performance-test "' + $runDirectory + '" --performance-iterations ' + $Iterations.ToString()
    if ($RapidSwitchProbe) { $argumentString += ' --rapid-switch-probe' }
    $process = Start-Process -FilePath $executable -ArgumentList $argumentString -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        try {
            $null = $process.CloseMainWindow()
            if (-not $process.WaitForExit(5000)) {
                $process.Kill()
                $null = $process.WaitForExit(10000)
            }
        } catch { }
        throw ('Performance run exceeded its ' + $TimeoutSeconds.ToString() + ' second bound. Run directory: ' + $runDirectory)
    }
    $process.Refresh()
    $reportPath = Join-Path $runDirectory 'performance.json'
    if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        $errorPath = Join-Path $runDirectory 'performance-startup-error.txt'
        if (Test-Path -LiteralPath $errorPath) { Get-Content -LiteralPath $errorPath -Encoding UTF8 }
        throw ('Performance process exited without a report (exit ' + $process.ExitCode + '). Run directory: ' + $runDirectory)
    }
    $report = Get-Content -LiteralPath $reportPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($process.ExitCode -ne 0 -or -not $report.succeeded) {
        throw ('Performance run failed. Exit=' + $process.ExitCode + '; error=' + [string]$report.error + '; report=' + $reportPath)
    }

    [pscustomobject]@{
        RunId = $report.runId
        ReportPath = $reportPath
        ExecutableSHA256 = $report.executableSha256
        FixtureSummary = (($report.fixtures | ForEach-Object { $_.Name + '=' + $_.LengthBytes + ' bytes' }) -join '; ')
        MeasuredSwitchSamples = @($report.samples | Where-Object { $_.category -eq 'measuredSwitch' }).Count
        Aggregates = $report.measuredSwitchAggregates
        RapidSwitchProbe = $report.rapidSwitchProbe
    } | Format-List
} finally {
    [Environment]::SetEnvironmentVariable('FOLIO_TEST_USER_DATA',$previousProfile,'Process')
    if ($null -ne $process) { $process.Dispose() }
}
