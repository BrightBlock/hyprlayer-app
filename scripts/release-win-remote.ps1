#requires -Version 5
# Keep this BOM-less script ASCII for PowerShell 5.1.
[CmdletBinding()]
param(
  [string] $SrcTarball = "$env:USERPROFILE\hyprlayer-src.tgz",
  [string] $BuildDir   = "$env:USERPROFILE\hyprlayer-release-build",
  [string] $TargetDir = "$env:USERPROFILE\.hyprlayer-win-target"
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Step($m) { Write-Host ("==> " + $m) -ForegroundColor Cyan }

function Run($cmdline) {
  cmd /c $cmdline
  if ($LASTEXITCODE -ne 0) { throw ("command failed (" + $LASTEXITCODE + "): " + $cmdline) }
}

if (-not (Test-Path $SrcTarball)) { throw ("source tarball not found: " + $SrcTarball) }

Step "Resetting build dir $BuildDir"
if (Test-Path $BuildDir) { Remove-Item -Recurse -Force $BuildDir }
New-Item -ItemType Directory -Force -Path $BuildDir | Out-Null

Step "Extracting source tarball"
Run ('tar -xzf "' + $SrcTarball + '" -C "' + $BuildDir + '"')

Set-Location $BuildDir

Step "Disabling updater artifacts (signing happens on the Mac)"
$confPath = (Resolve-Path 'src-tauri\tauri.conf.json').ProviderPath
$txt = Get-Content $confPath -Raw
$new = $txt -replace '"createUpdaterArtifacts"\s*:\s*true', '"createUpdaterArtifacts": false'
if ($new -eq $txt) {
  Write-Warning "createUpdaterArtifacts:true not found; build will need a signing key and likely fail"
}
[System.IO.File]::WriteAllText($confPath, $new, (New-Object System.Text.UTF8Encoding($false)))

Step "pnpm install (frozen lockfile)"
Run 'pnpm install --frozen-lockfile'

if (-not $env:RC) {
  $rc = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Recurse -Filter rc.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -like '*\x64\*' } |
    Sort-Object FullName -Descending | Select-Object -First 1
  if ($rc) { $env:RC = $rc.FullName; Step ("Pinned RC=" + $env:RC) }
  else { Write-Warning "rc.exe not found under Windows Kits; winres may fail" }
}

Step "tauri build --bundles nsis"
$env:CARGO_TARGET_DIR = $TargetDir
Run 'pnpm tauri build --bundles nsis'

Step "Locating installer"
$nsisDir = Join-Path $TargetDir 'release\bundle\nsis'
$setup = Get-ChildItem -Path $nsisDir -Filter '*-setup.exe' -ErrorAction Stop |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { throw ("no *-setup.exe found under " + $nsisDir) }

Write-Host ""
Write-Host ("SETUP_EXE=" + $setup.FullName)
