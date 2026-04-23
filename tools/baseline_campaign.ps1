param(
    [ValidateRange(1, 200)]
    [int]$Boots = 10,
    [ValidateRange(1, 200)]
    [int]$MinConsecutivePass = 10,
    [string]$KernelFeatures = "baseline",
    [ValidateRange(5, 600)]
    [int]$RunTimeoutSec = 20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$msg) {
    Write-Error $msg
    exit 1
}

function Count-Matches([string]$text, [string]$pattern) {
    return ([regex]::Matches($text, $pattern)).Count
}

function Test-TickMonotonic([string]$text) {
    $matches = [regex]::Matches($text, 'R10 tick=(\d+)')
    if ($matches.Count -eq 0) { return $false }

    $prev = -1L
    foreach ($m in $matches) {
        $cur = [long]$m.Groups[1].Value
        if ($prev -ge 0 -and $cur -le $prev) { return $false }
        $prev = $cur
    }
    return $true
}

function Get-R10Stats([string]$text) {
    $matches = [regex]::Matches($text, 'R10 tick=\d+\s+dt=(\d+)\s+miss=(\d+)')
    $maxDt = 0
    $maxMiss = 0
    foreach ($m in $matches) {
        $dt = [int]$m.Groups[1].Value
        $miss = [int]$m.Groups[2].Value
        if ($dt -gt $maxDt) { $maxDt = $dt }
        if ($miss -gt $maxMiss) { $maxMiss = $miss }
    }
    return [PSCustomObject]@{
        samples = $matches.Count
        max_dt = $maxDt
        max_miss = $maxMiss
    }
}

function Test-ResetOrSilentFault([string]$text) {
    $bootRepeatAfterRuntime = [regex]::IsMatch($text, 'R00 runtime start[\s\S]*Prometheus Bootloader')
    $hasResetHint = [regex]::IsMatch($text.ToLowerInvariant(), 'triple fault|guest reset|watchdog reset|rebooting|resetting')
    return ($bootRepeatAfterRuntime -or $hasResetHint)
}

function Test-BaselineLog([string]$rawLogPath) {
    if (!(Test-Path $rawLogPath)) {
        return [PSCustomObject]@{
            pass = $false
            reason = "raw log missing"
            required = @{}
            errors = @{}
            tick_monotonic = $false
            r10_stats = [PSCustomObject]@{ samples = 0; max_dt = 0; max_miss = 0 }
            reset_or_silent_fault = $true
        }
    }

    $text = [System.IO.File]::ReadAllText($rawLogPath)
    $r10stats = Get-R10Stats $text

    $required = [ordered]@{
        bootloader = $text.Contains("Prometheus Bootloader")
        gop = $text.Contains("GOP OK")
        disk = $text.Contains("Disk OK")
        kernel_loaded = $text.Contains("Kernel loaded")
        exit_bs = $text.Contains("ExitBootServices OK")
        jump = $text.Contains("Jump to kernel")
        k00 = $text.Contains("K00 kernel entry")
        k07 = $text.Contains("K07 fabric done")
        r00 = $text.Contains("R00 runtime start")
        r10 = $text.Contains("R10 tick=")
        s10 = $text.Contains("S10 cur=")
    }

    $errors = [ordered]@{
        E20 = Count-Matches $text 'E20\s+ovf'
        E30 = Count-Matches $text 'E30\s+evq_ovf'
        E31 = Count-Matches $text 'E31\s+starve'
        E40 = Count-Matches $text 'E40\s+task_ovf_seen'
        E50 = Count-Matches $text 'E50\s+dl_viol'
        E51 = Count-Matches $text 'E51\s+dl_sum'
        panic = Count-Matches $text 'P00\s+panic|\bpanic\b'
        exc = Count-Matches $text 'XUD|XGP|XPF|XDF'
    }

    $tickOk = Test-TickMonotonic $text
    $resetOrSilent = Test-ResetOrSilentFault $text
    $requiredOk = -not ($required.GetEnumerator() | Where-Object { -not $_.Value })
    $errorsOk = (
        $errors.E20 -eq 0 -and
        $errors.E30 -eq 0 -and
        $errors.E31 -eq 0 -and
        $errors.E40 -eq 0 -and
        $errors.E50 -eq 0 -and
        $errors.E51 -eq 0 -and
        $errors.panic -eq 0 -and
        $errors.exc -eq 0
    )

    $pass = $requiredOk -and $errorsOk -and $tickOk -and (-not $resetOrSilent)
    $reason = if ($pass) { "ok" } else { "required/error/tick/reset violation" }

    return [PSCustomObject]@{
        pass = $pass
        reason = $reason
        required = $required
        errors = $errors
        tick_monotonic = $tickOk
        r10_stats = $r10stats
        reset_or_silent_fault = $resetOrSilent
    }
}

$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = Split-Path -Parent $toolsDir
$runScript = Join-Path $root "run.ps1"
if (!(Test-Path $runScript)) { Fail "run.ps1 not found: $runScript" }
if ($Boots -lt $MinConsecutivePass) { Fail "Boots must be >= MinConsecutivePass" }

$baselineDir = Join-Path $root "dist\logs\baseline"
New-Item -ItemType Directory -Force $baselineDir | Out-Null
Get-ChildItem $baselineDir -File -Filter "boot_*.raw.log" -ErrorAction SilentlyContinue | Remove-Item -Force
Get-ChildItem $baselineDir -File -Filter "boot_*.report.json" -ErrorAction SilentlyContinue | Remove-Item -Force
$summaryPath = Join-Path $baselineDir "summary.json"
if (Test-Path $summaryPath) { Remove-Item $summaryPath -Force }

$originalFeatures = $env:PROMETHEUS_KERNEL_FEATURES
$env:PROMETHEUS_KERNEL_FEATURES = $KernelFeatures

$results = @()
$consecutive = 0
$maxConsecutive = 0

try {
    for ($i = 1; $i -le $Boots; $i++) {
        $id = ("{0:D3}" -f $i)
        Write-Host ("[campaign {0}/{1}] run.ps1" -f $i, $Boots)

        Get-Process qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

        $runProc = Start-Process -FilePath "powershell" -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $runScript) -PassThru -WindowStyle Hidden
        $timedOut = $false
        try {
            Wait-Process -Id $runProc.Id -Timeout $RunTimeoutSec -ErrorAction Stop
        }
        catch {
            $timedOut = $true
        }

        if ($timedOut) {
            Get-Process qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
            Stop-Process -Id $runProc.Id -Force -ErrorAction SilentlyContinue
            $runExit = 124
        } else {
            $runExit = $runProc.ExitCode
        }

        $latestJsonPath = Join-Path $root "dist\logs\latest.json"
        if (!(Test-Path $latestJsonPath)) { Fail "latest.json missing after run #$i" }

        $latest = Get-Content $latestJsonPath -Raw | ConvertFrom-Json
        $rawPath = [string]$latest.raw_log
        if ([string]::IsNullOrWhiteSpace($rawPath) -or !(Test-Path $rawPath)) {
            Fail "latest.json points to missing raw_log after run #${i}: $rawPath"
        }

        $dstRaw = Join-Path $baselineDir ("boot_" + $id + ".raw.log")
        Copy-Item $rawPath $dstRaw -Force

        $analysis = Test-BaselineLog $dstRaw
        $runPass = (($runExit -eq 0) -or ($runExit -eq 124)) -and $analysis.pass

        if ($runPass) {
            $consecutive += 1
            if ($consecutive -gt $maxConsecutive) { $maxConsecutive = $consecutive }
        } else {
            $consecutive = 0
        }

        $report = [PSCustomObject]@{
            boot_id = $id
            pass = $runPass
            run_exit_code = $runExit
            raw_log = $dstRaw
            latest_run_id = [string]$latest.run_id
            latest_stamp = [string]$latest.stamp
            checks = $analysis
        }

        $reportPath = Join-Path $baselineDir ("boot_" + $id + ".report.json")
        $report | ConvertTo-Json -Depth 8 | Set-Content -Path $reportPath -Encoding UTF8
        $results += $report

        Write-Host ("[campaign {0}/{1}] result: {2}" -f $i, $Boots, ($(if($runPass){'PASS'}else{'FAIL'})))
    }
}
finally {
    if ($null -eq $originalFeatures) {
        Remove-Item Env:PROMETHEUS_KERNEL_FEATURES -ErrorAction SilentlyContinue
    } else {
        $env:PROMETHEUS_KERNEL_FEATURES = $originalFeatures
    }
}

$passCount = @($results | Where-Object { $_.pass }).Count
$failCount = $results.Count - $passCount
$campaignPass = ($maxConsecutive -ge $MinConsecutivePass) -and ($failCount -eq 0)

$failRuns = @($results | Where-Object { -not $_.pass } | ForEach-Object {
    [PSCustomObject]@{
        boot_id = $_.boot_id
        run_exit_code = $_.run_exit_code
        reason = $_.checks.reason
        raw_log = $_.raw_log
    }
})

$maxDt = 0
$maxMiss = 0
foreach ($r in $results) {
    if ($null -ne $r.checks -and $null -ne $r.checks.r10_stats) {
        if ([int]$r.checks.r10_stats.max_dt -gt $maxDt) { $maxDt = [int]$r.checks.r10_stats.max_dt }
        if ([int]$r.checks.r10_stats.max_miss -gt $maxMiss) { $maxMiss = [int]$r.checks.r10_stats.max_miss }
    }
}

$summary = [PSCustomObject]@{
    generated_at = (Get-Date).ToString("o")
    root = $root
    mode = "baseline_campaign"
    kernel_features = $KernelFeatures
    run_timeout_sec = $RunTimeoutSec
    boots_requested = $Boots
    min_consecutive_pass = $MinConsecutivePass
    boots_pass = $passCount
    boots_fail = $failCount
    max_consecutive_pass = $maxConsecutive
    consecutive_pass = $maxConsecutive
    campaign_pass = $campaignPass
    fails = $failRuns
    aggregates = [PSCustomObject]@{ max_dt = $maxDt; max_miss = $maxMiss }
    runs = $results
}

$summary | ConvertTo-Json -Depth 10 | Set-Content -Path $summaryPath -Encoding UTF8
Write-Host ("Campaign complete: {0}" -f ($(if($campaignPass){'PASS'}else{'FAIL'})))
Write-Host ("Summary: {0}" -f $summaryPath)

if (-not $campaignPass) { exit 2 }
exit 0


