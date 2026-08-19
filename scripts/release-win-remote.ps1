#requires -Version 5
<#
  Build the Windows x64 NSIS installer on this machine (the "nexus-win" builder,
  a Windows guest hosted on nexus),
  driven over SSH by scripts/release-local.sh on the Mac.

  The Mac exports the exact release-tag source with 'git archive' and ships it
  here as a tarball, so this build matches what the Mac publishes byte for byte,
  independent of any working clone on this box. We build in a clean throwaway
  dir and never touch the developer's own checkout.

  Updater artifacts are deliberately disabled: the ed25519 updater signing key
  stays on the Mac, which signs the copied-down installer itself. This machine
  only needs to produce the raw <name>-setup.exe.

  On success the LAST line printed is "SETUP_EXE=<full path>", which the Mac
  orchestrator parses to know what to copy back.

  NOTE: keep this file pure ASCII. PowerShell 5.1 reads BOM-less scripts as ANSI,
  so any non-ASCII byte (em dash, smart quote) corrupts the parser.

  Invoked from the Mac as (paths default under the builder's home dir, so the
  orchestrator just scps the tarball to ~ and runs this with no path args):
    powershell -NoProfile -ExecutionPolicy Bypass -File %USERPROFILE%\release-win-remote.ps1
#>
[CmdletBinding()]
param(
  # Source tarball the Mac scp'd into the builder's home dir, and a throwaway
  # build dir. Defaults keep the Mac side from having to resolve the remote home.
  [string] $SrcTarball = "$env:USERPROFILE\hyprlayer-src.tgz",
  [string] $BuildDir   = "$env:USERPROFILE\hyprlayer-release-build",
  # Persistent cargo target dir (outside BuildDir, which is wiped each run) so
  # crate compilation is cached across releases; only the app's own crates
  # recompile when the source changes.
  [string] $TargetDir = "$env:USERPROFILE\.hyprlayer-win-target"
)

$ErrorActionPreference = 'Stop'
# Keep pnpm / cargo / tauri progress bars from flooding the SSH pipe.
$ProgressPreference = 'SilentlyContinue'

function Step($m) { Write-Host ("==> " + $m) -ForegroundColor Cyan }

# Run a build tool through cmd so its exit code is reliable and its stderr does
# not trip $ErrorActionPreference. Pass the full command line as one string.
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
# Surgical text replace rather than a JSON round-trip: PowerShell 5.1's
# ConvertTo-Json reorders keys, caps depth, and re-escapes content, which would
# corrupt the CSP and other nested config. A literal swap is safe.
$confPath = (Resolve-Path 'src-tauri\tauri.conf.json').ProviderPath
$txt = Get-Content $confPath -Raw
$new = $txt -replace '"createUpdaterArtifacts"\s*:\s*true', '"createUpdaterArtifacts": false'
if ($new -eq $txt) {
  Write-Warning "createUpdaterArtifacts:true not found; build will need a signing key and likely fail"
}
# Write UTF-8 without BOM (PS 5.1 'utf8' adds a BOM, which can break parsers).
[System.IO.File]::WriteAllText($confPath, $new, (New-Object System.Text.UTF8Encoding($false)))

Step "pnpm install (frozen lockfile)"
Run 'pnpm install --frozen-lockfile'

# tauri-winres shells out to the Windows SDK resource compiler (rc.exe) to embed
# the app icon/version. Over a bare SSH session the SDK bin is not always on PATH,
# so the build script panics with "have RC.EXE in your $PATH ... or $RC is set?".
# Pin $RC to the newest x64 rc.exe so the build is reproducible regardless of PATH.
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
