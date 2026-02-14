Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$msg) {
    Write-Error $msg
    exit 1
}

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$dstDir = Join-Path $root "dist\ovmf"
$dstVars = Join-Path $dstDir "edk2-x86_64-vars.fd"
$altVars = Join-Path $dstDir "OVMF_VARS.fd"

if (Test-Path $dstVars) {
    Write-Host "OVMF VARS already present: $dstVars"
    exit 0
}
if (Test-Path $altVars) {
    Copy-Item $altVars $dstVars -Force
    Write-Host "OVMF VARS prepared: $dstVars"
    exit 0
}

New-Item -ItemType Directory -Force $dstDir | Out-Null
$tmpDir = Join-Path $dstDir ".tmp"
New-Item -ItemType Directory -Force $tmpDir | Out-Null
$rpmPath = Join-Path $tmpDir "edk2-ovmf.rpm"

$urls = @(
    "http://ftp.icm.edu.pl/pub/Linux/dist/fedora/linux/releases/42/Everything/x86_64/os/Packages/e/edk2-ovmf-20250221-8.fc42.noarch.rpm"
)

$downloaded = $false
foreach ($url in $urls) {
    try {
        Invoke-WebRequest -Uri $url -OutFile $rpmPath -UseBasicParsing
        $downloaded = $true
        break
    } catch {
    }
}

if (-not $downloaded) {
    Fail "Нет OVMF VARS. Укажи `$env:OVMF_VARS=... или запусти tools\fetch_ovmf_vars.ps1"
}

$extractDir = Join-Path $tmpDir "extract"
New-Item -ItemType Directory -Force $extractDir | Out-Null

try {
    tar -xf $rpmPath -C $extractDir
} catch {
    Fail "Нет OVMF VARS. Укажи `$env:OVMF_VARS=... или запусти tools\fetch_ovmf_vars.ps1"
}

$candidates = @(
    @(Get-ChildItem $extractDir -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'x86_64.*vars.*\.fd' } | Select-Object -ExpandProperty FullName),
    @(Get-ChildItem $extractDir -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'OVMF_VARS\.fd|OVMF_VARS\.4m\.fd' } | Select-Object -ExpandProperty FullName)
)

$srcVars = $null
foreach ($group in $candidates) {
    foreach ($p in $group) {
        if (Test-Path $p) {
            $srcVars = $p
            break
        }
    }
    if ($srcVars) { break }
}

if (-not $srcVars) {
    Fail "Нет OVMF VARS. Укажи `$env:OVMF_VARS=... или запусти tools\fetch_ovmf_vars.ps1"
}

Copy-Item $srcVars $dstVars -Force
Write-Host "OVMF VARS saved: $dstVars"
