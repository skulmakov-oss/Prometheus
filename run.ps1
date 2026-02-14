Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$msg) {
    Write-Error $msg
    exit 1
}

function Resolve-ExistingPath([string[]]$candidates) {
    foreach ($p in $candidates) {
        if ([string]::IsNullOrWhiteSpace($p)) { continue }
        if (Test-Path $p) { return (Resolve-Path $p).Path }
    }
    return $null
}

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $root

Write-Host "[1/5] Build bootloader (release)"
cargo build --release
if ($LASTEXITCODE -ne 0) { Fail "bootloader build failed" }

Write-Host "[2/5] Build kernel (release)"
Push-Location ".\kernel"
cargo build --release
if ($LASTEXITCODE -ne 0) { Pop-Location; Fail "kernel build failed" }
Pop-Location

$efiSrc = Join-Path $root "target\x86_64-unknown-uefi\release\vectoros-bootloader.efi"
$elfSrc = Join-Path $root "kernel\target\x86_64-unknown-none\release\vectoros-kernel"
if (!(Test-Path $efiSrc)) { Fail "missing bootloader EFI: $efiSrc" }
if (!(Test-Path $elfSrc)) { Fail "missing kernel ELF: $elfSrc" }

Write-Host "[3/5] Prepare ESP folder"
$bootDir = Join-Path $root "dist\esp\EFI\BOOT"
New-Item -ItemType Directory -Force $bootDir | Out-Null
Copy-Item $efiSrc (Join-Path $bootDir "BOOTX64.EFI") -Force
Copy-Item $elfSrc (Join-Path $bootDir "KERNEL.ELF") -Force

$qemu = $null
if (-not [string]::IsNullOrWhiteSpace($env:QEMU_EXE)) {
    if (Test-Path $env:QEMU_EXE) {
        $qemu = (Resolve-Path $env:QEMU_EXE).Path
    } else {
        Fail "QEMU_EXE points to missing file: $($env:QEMU_EXE)"
    }
} else {
    $cmd = Get-Command "qemu-system-x86_64" -ErrorAction SilentlyContinue
    if ($cmd) {
        $qemu = $cmd.Source
    } elseif (Test-Path "C:\Program Files\qemu\qemu-system-x86_64.exe") {
        $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
    }
}
if ($null -eq $qemu) {
    Fail "qemu-system-x86_64 not found. Hint: winget install --id SoftwareFreedomConservancy.QEMU -e"
}

$shareDir = "C:\Program Files\qemu\share"
$ovmfCode = $null
if (-not [string]::IsNullOrWhiteSpace($env:OVMF_CODE)) {
    if (Test-Path $env:OVMF_CODE) {
        $ovmfCode = (Resolve-Path $env:OVMF_CODE).Path
    } else {
        Fail "OVMF_CODE points to missing file: $($env:OVMF_CODE)"
    }
} else {
    $defaultCode = Join-Path $shareDir "edk2-x86_64-code.fd"
    if (Test-Path $defaultCode) {
        $ovmfCode = $defaultCode
    } else {
        $ovmfCode = Resolve-ExistingPath @(
            (Get-ChildItem $shareDir -File -Filter "*x86_64*code*.fd" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName)
        )
    }
}
if ($null -eq $ovmfCode) {
    Fail "x86_64 OVMF CODE not found under $shareDir (set OVMF_CODE manually)."
}

$ovmfVars = $null
if (-not [string]::IsNullOrWhiteSpace($env:OVMF_VARS)) {
    if (Test-Path $env:OVMF_VARS) {
        $ovmfVars = (Resolve-Path $env:OVMF_VARS).Path
    } else {
        Fail "OVMF_VARS points to missing file: $($env:OVMF_VARS)"
    }
} else {
    $defaultVars = Join-Path $shareDir "edk2-x86_64-vars.fd"
    if (Test-Path $defaultVars) {
        $ovmfVars = $defaultVars
    } else {
        $fetchScript = Join-Path $root "tools\fetch_ovmf_vars.ps1"
        if (Test-Path $fetchScript) {
            & powershell -ExecutionPolicy Bypass -File $fetchScript
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        }
        $ovmfVars = Resolve-ExistingPath @(
            (Join-Path $root "dist\ovmf\edk2-x86_64-vars.fd")
        )
    }
}
if ($null -eq $ovmfVars) {
    Write-Host "Нет OVMF VARS. Укажи `$env:OVMF_VARS=... или запусти tools\fetch_ovmf_vars.ps1"
    exit 1
}

$ovmfDir = Join-Path $root "dist\ovmf"
New-Item -ItemType Directory -Force $ovmfDir | Out-Null
$varsRuntime = Join-Path $ovmfDir "vars-runtime.fd"
if (!(Test-Path $varsRuntime)) {
    Copy-Item $ovmfVars $varsRuntime -Force
}

$espDir = (Resolve-Path (Join-Path $root "dist\esp")).Path
$espDirQemu = $espDir -replace '\\','/'
$logDir = Join-Path $root "dist\logs"
New-Item -ItemType Directory -Force $logDir | Out-Null
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$rawLogPath = Join-Path $logDir ("run-" + $stamp + ".raw.log")
$jsonLogPath = Join-Path $logDir ("run-" + $stamp + ".json")
$latestJsonPath = Join-Path $logDir "latest.json"
Write-Host "[4/5] Launch QEMU"
Write-Host "QEMU: $qemu"
Write-Host "OVMF_CODE: $ovmfCode"
Write-Host "OVMF_VARS: $varsRuntime"
Write-Host "ESP: $espDirQemu"
Write-Host "LOG_JSON: $jsonLogPath"

$qemuOutput = & $qemu `
    -machine q35 `
    -m 512M `
    -serial stdio `
    -drive if=pflash,format=raw,readonly=on,file=$ovmfCode `
    -drive if=pflash,format=raw,file=$varsRuntime `
    -drive if=virtio,format=raw,file="fat:rw:$espDirQemu" 2>&1 | Tee-Object -FilePath $rawLogPath

$exitCode = $LASTEXITCODE
$lines = @($qemuOutput | ForEach-Object { $_.ToString() })

$logObject = [PSCustomObject]@{
    timestamp = (Get-Date).ToString("o")
    qemu = $qemu
    ovmf_code = $ovmfCode
    ovmf_vars = $varsRuntime
    esp = $espDirQemu
    exit_code = $exitCode
    lines = $lines
}
$logObject | ConvertTo-Json -Depth 5 | Set-Content $jsonLogPath
Copy-Item $jsonLogPath $latestJsonPath -Force

Write-Host "[5/5] QEMU exit code: $exitCode"
Write-Host "Last log lines:"
$tail = @(
    $lines |
    ForEach-Object { $_ -replace "\x1b\[[0-9;?=]*[A-Za-z]", "" } |
    Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
    Select-Object -Last 20
)
if ($tail.Count -eq 0) {
    Write-Host "(no output)"
} else {
    $tail | ForEach-Object { Write-Host $_ }
}
exit $exitCode
