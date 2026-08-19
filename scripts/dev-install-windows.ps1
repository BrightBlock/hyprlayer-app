<#
.SYNOPSIS
  Build and install "Hyprlayer Dev" on Windows: a desktop install that tracks
  origin/main, SIDE BY SIDE with the released app the NSIS installer owns.
  (macOS: scripts/dev-install-macos.sh. Linux: scripts/dev-install-linux.sh.)

.DESCRIPTION
  Why a separate install rather than 'pnpm tauri dev':
    * it survives a reboot and shows up in the Start menu and on PATH, so main
      can be dogfooded all day instead of only while a terminal holds the dev
      server
    * it is a release-profile build (no vite dev server, no HMR), so what you
      are testing is what a release would feel like

  Three things keep it from colliding with the released install:
    * identifier  ai.brightblock.hyprlayer.main  -> its own SQLite DB, settings
      and WebView2 profile under AppData. Nothing this build migrates can
      corrupt the stable app's database.
    * it installs under %LOCALAPPDATA%\Programs, never Program Files, so the
      NSIS installer still owns the release and no elevation is needed
    * a distinct executable name (Hyprlayer-Dev.exe) at a distinct path, so the
      taskbar and Alt-Tab treat the two as different apps

  The identifier says ".main" while the app is called "Hyprlayer Dev" on
  purpose: ".dev" already belongs to the 'verify' skill, which launches
  'tauri dev --config src-tauri/tauri.dev.conf.json' under
  ai.brightblock.hyprlayer.dev. Sharing that identifier would let a running
  install hijack a verify run through the single-instance plugin, so this build
  gets its own, named after the branch it tracks.

  What lands where:
    exe       %LOCALAPPDATA%\Programs\Hyprlayer-Dev\Hyprlayer-Dev.exe
              (that directory is added to the user PATH, so 'Hyprlayer-Dev'
              launches it from any shell)
    shortcut  %APPDATA%\Microsoft\Windows\Start Menu\Programs\Hyprlayer Dev.lnk
    data      %APPDATA%\ai.brightblock.hyprlayer.main
    webview   %LOCALAPPDATA%\ai.brightblock.hyprlayer.main\EBWebView

  The exe is named Hyprlayer-Dev.exe rather than Hyprlayer.exe because PATH is
  also where the app's own binary resolver looks for the hyprlayer CLI, and
  that lookup is case-insensitive on Windows (see the version gate in
  crates/hyprlayer-core/src/process/binaries.rs). A GUI Hyprlayer.exe on PATH
  is exactly the shape that has been mistaken for the CLI before.

  The build happens in a throwaway clone under %USERPROFILE% (never this
  working tree), so it can run while you have uncommitted work in progress, and
  it always builds exactly what is on the remote rather than whatever you had
  checked out. Paths stay short on purpose: a deep node_modules or cargo tree
  under a long prefix still trips MAX_PATH on stock Windows. The cargo target
  dir is persistent, so only the first build is cold; after that it is an
  incremental rebuild of the app's own crates.

  Signing: this build is deliberately left UNSIGNED, so SmartScreen may warn
  the first time you launch it ("More info" -> "Run anyway"). Updater artifacts
  are off for the same reason: signing those needs the ed25519 release key.

  Updater note: the in-app updater on Windows installs an update passively, but
  the +main.<sha> version below is not a release version, and semver ignores
  build metadata, so the manifest's x.y.z never compares as newer and the nudge
  stays quiet. That suffix is also what tells you which commit you are on
  (Settings -> Updates reads e.g. 0.5.10+main.d39276a). Pass -NoVersionSuffix
  for a bare x.y.z.

  Prerequisites: the same toolchain the Windows release builder needs, minus
  every secret: Rust (MSVC toolchain), Node + pnpm, the Windows SDK (for
  rc.exe) and the WebView2 runtime. No signing key, no publish credentials.

  NOTE: keep this file pure ASCII. PowerShell 5.1 reads BOM-less scripts as
  ANSI, so any non-ASCII byte (em dash, smart quote) corrupts the parser.

.PARAMETER Ref
  Track something else than origin/main (a branch, tag or SHA).

.PARAMETER Force
  Rebuild even if the tracked ref has not moved.

.PARAMETER NoFetch
  Use what is already fetched into the build clone (offline).

.PARAMETER NoVersionSuffix
  Build a bare x.y.z instead of x.y.z+main.<sha>.

.PARAMETER Uninstall
  Remove the install (keeps its data dir).

.EXAMPLE
  .\scripts\dev-install-windows.ps1
  Fetch origin/main, build if it moved, install.

.EXAMPLE
  .\scripts\dev-install-windows.ps1 -Ref feature/my-branch -Force
  Build a specific ref even if the stamp says it is current.
#>

#requires -Version 5

[CmdletBinding()]
param(
  [string] $Ref = 'origin/main',
  [switch] $Force,
  [switch] $NoFetch,
  [switch] $NoVersionSuffix,
  [switch] $Uninstall
)

$ErrorActionPreference = 'Stop'
# Keep pnpm / cargo / tauri progress bars from flooding an SSH pipe.
$ProgressPreference = 'SilentlyContinue'

if ($env:OS -ne 'Windows_NT') {
  throw 'this is the Windows installer; use scripts/dev-install-macos.sh or scripts/dev-install-linux.sh'
}

$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).ProviderPath

# --- config --------------------------------------------------------
$CacheDir  = if ($env:HYPRLAYER_DEV_CACHE) { $env:HYPRLAYER_DEV_CACHE } else { Join-Path $env:USERPROFILE '.hyprlayer-dev' }
$SrcDir    = Join-Path $CacheDir 'src'
$TargetDir = Join-Path $CacheDir 'target'
$Stamp     = Join-Path $CacheDir 'installed'

$Prefix   = if ($env:HYPRLAYER_DEV_PREFIX) { $env:HYPRLAYER_DEV_PREFIX } else { Join-Path $env:LOCALAPPDATA 'Programs' }
$AppId    = 'ai.brightblock.hyprlayer.main'
$AppName  = 'Hyprlayer Dev'
# The executable name doubles as the PATH command and the install dir name.
$BinName  = 'Hyprlayer-Dev'

$InstallDir   = Join-Path $Prefix $BinName
$InstalledExe = Join-Path $InstallDir "$BinName.exe"
$StartMenu    = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$Shortcut     = Join-Path $StartMenu "$AppName.lnk"
$DataDir      = Join-Path $env:APPDATA $AppId
$WebviewDir   = Join-Path $env:LOCALAPPDATA $AppId

function Step($m) { Write-Host ('==> ' + $m) -ForegroundColor Cyan }
function Info($m) { Write-Host ('    ' + $m) }
function Warn($m) { Write-Warning $m }

# Run a build tool through cmd so its exit code is reliable and its stderr does
# not trip $ErrorActionPreference. Pass the full command line as one string.
function Run($cmdline) {
  cmd /c $cmdline
  if ($LASTEXITCODE -ne 0) { throw ('command failed (' + $LASTEXITCODE + '): ' + $cmdline) }
}

# Named RunGit, not Git: PowerShell resolves a function before an executable
# of the same name, so a 'Git' function calling 'git' recurses until the stack
# overflows. git is called directly rather than through Run: it is a real exe (unlike the
# pnpm .cmd shim) so its exit code is already reliable, and an argument array
# dodges cmd's '^' and '%' escaping, which would mangle 'origin/main^{commit}'.
# The local $ErrorActionPreference is what makes 2>&1 safe: at 'Stop', anything
# a native command writes to a redirected stderr becomes a terminating error,
# and git narrates plenty on stderr while succeeding.
function RunGit([string[]] $GitArgs, [switch] $Quiet) {
  $ErrorActionPreference = 'Continue'
  $out = & git @GitArgs 2>&1
  if ($LASTEXITCODE -ne 0) {
    if ($Quiet) { return $null }
    throw ('git ' + ($GitArgs -join ' ') + ' failed: ' + (($out | Out-String).Trim()))
  }
  return @($out | Where-Object { $_ -isnot [System.Management.Automation.ErrorRecord] })
}

# First line of a git command's output, or $null if it failed. The @() is load
# bearing: PowerShell unwraps a one-element array on return, so a bare $out[0]
# indexes the first CHARACTER of a single-line result.
function GitLine([string[]] $GitArgs) {
  $out = RunGit $GitArgs -Quiet
  if ($null -eq $out) { return $null }
  $lines = @($out)
  if ($lines.Count -eq 0) { return $null }
  return ([string]$lines[0]).Trim()
}

# --- user PATH -----------------------------------------------------
# Read and write the raw registry value rather than going through
# [Environment]::GetEnvironmentVariable('Path','User'): that expands a
# REG_EXPAND_SZ Path, and SetEnvironmentVariable would then bake the expansion
# back in as a REG_SZ, permanently flattening entries like %USERPROFILE%\bin.
function GetUserPath {
  $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $false)
  try {
    if (-not $key) { return @{ Value = ''; Kind = [Microsoft.Win32.RegistryValueKind]::ExpandString } }
    $raw = $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    if ($null -eq $raw) { return @{ Value = ''; Kind = [Microsoft.Win32.RegistryValueKind]::ExpandString } }
    return @{ Value = [string]$raw; Kind = $key.GetValueKind('Path') }
  } finally { if ($key) { $key.Close() } }
}

function SetUserPath($value, $kind) {
  $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
  try { $key.SetValue('Path', $value, $kind) } finally { if ($key) { $key.Close() } }
  # Without the broadcast, only processes started after the next sign-in see the
  # change: explorer.exe hands its own stale environment to everything it spawns.
  try {
    if (-not ('Win32.EnvBroadcast' -as [type])) {
      Add-Type -Namespace Win32 -Name EnvBroadcast -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Auto)]
public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);
'@
    }
    $result = [System.UIntPtr]::Zero
    # HWND_BROADCAST, WM_SETTINGCHANGE, SMTO_ABORTIFHUNG, 5s.
    [Win32.EnvBroadcast]::SendMessageTimeout([System.IntPtr]0xffff, 0x1A, [System.UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$result) | Out-Null
  } catch {
    Warn 'could not broadcast the PATH change; open a new terminal or sign out and back in'
  }
}

function PathEntries($value) {
  return @($value -split ';' | Where-Object { $_ -ne '' })
}

function AddToUserPath($dir) {
  $current = GetUserPath
  $entries = PathEntries $current.Value
  # Compare expanded, so an existing %LOCALAPPDATA%\Programs\... entry counts.
  $already = $entries | Where-Object {
    [System.Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -ieq $dir.TrimEnd('\')
  }
  if ($already) { return $false }
  $joined = (@($entries) + $dir) -join ';'
  SetUserPath $joined $current.Kind
  return $true
}

function RemoveFromUserPath($dir) {
  $current = GetUserPath
  $entries = PathEntries $current.Value
  $kept = @($entries | Where-Object {
    [System.Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -ine $dir.TrimEnd('\')
  })
  if ($kept.Count -eq $entries.Count) { return $false }
  SetUserPath ($kept -join ';') $current.Kind
  return $true
}

# --- uninstall -----------------------------------------------------
if ($Uninstall) {
  Step "Removing $AppName"
  foreach ($p in @($InstallDir, $Shortcut, $Stamp)) {
    if (Test-Path $p) { Remove-Item -Recurse -Force $p; Info "removed $p" }
  }
  if (RemoveFromUserPath $InstallDir) { Info "removed $InstallDir from the user PATH" }
  Info 'Kept (delete by hand if you want a clean slate):'
  Info "  build cache  $CacheDir"
  Info "  app data     $DataDir"
  Info "  webview      $WebviewDir"
  exit 0
}

foreach ($tool in @('git', 'cargo', 'pnpm', 'node')) {
  if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
    throw "missing required tool: $tool"
  }
}

# --- 1. a throwaway clone that tracks the remote -------------------
# Cloned from the local repo (fast, no network for the object transfer), then
# repointed at the real origin so 'origin/main' means the same thing here as it
# does in the working tree.
if (-not (Test-Path (Join-Path $SrcDir '.git'))) {
  Step "Creating build clone at $SrcDir"
  New-Item -ItemType Directory -Force -Path $CacheDir | Out-Null
  if (Test-Path $SrcDir) { Remove-Item -Recurse -Force $SrcDir }
  RunGit @('clone', '--quiet', $Repo, $SrcDir) | Out-Null
  $originUrl = GitLine @('-C', $Repo, 'remote', 'get-url', 'origin')
  if ($originUrl) { RunGit @('-C', $SrcDir, 'remote', 'set-url', 'origin', $originUrl) | Out-Null }
}

if (-not $NoFetch) {
  Step "Fetching $Ref"
  try { RunGit @('-C', $SrcDir, 'fetch', '--quiet', '--prune', 'origin') | Out-Null }
  catch { throw ('fetch failed (try -NoFetch to build what is already here): ' + $_.Exception.Message) }
}

$Sha = GitLine @('-C', $SrcDir, 'rev-parse', '--verify', "$Ref^{commit}")
if (-not $Sha) { throw "no such ref in the build clone: $Ref" }
$Short   = GitLine @('-C', $SrcDir, 'rev-parse', '--short', $Sha)
$Subject = GitLine @('-C', $SrcDir, 'log', '-1', '--format=%s', $Sha)

$StampSha = if (Test-Path $Stamp) { ([string](Get-Content $Stamp -Raw)).Trim() } else { '' }
if ((-not $Force) -and (Test-Path $InstalledExe) -and $StampSha -eq $Sha) {
  Step "Already on $Ref ($Short - $Subject)"
  Info 'Nothing to build. Use -Force to rebuild anyway.'
  exit 0
}

# --- 2. check out the exact commit ---------------------------------
# 'clean -qfd' (no -x) drops stray tracked-turned-untracked files but keeps the
# gitignored node_modules/ and dist/, which is what makes reruns cheap.
Step "Checking out $Short - $Subject"
RunGit @('-C', $SrcDir, 'checkout', '--quiet', '--detach', $Sha) | Out-Null
RunGit @('-C', $SrcDir, 'reset', '--hard', '--quiet', $Sha) | Out-Null
RunGit @('-C', $SrcDir, 'clean', '-qfd') | Out-Null

Set-Location $SrcDir

# --- 3. config overlay ---------------------------------------------
# Written into the build clone (not this repo) so nothing here needs
# committing. createUpdaterArtifacts is forced off: signing them needs the
# release key.
$Version = (Get-Content (Join-Path $SrcDir 'package.json') -Raw | ConvertFrom-Json).version
$BuildVersion = if ($NoVersionSuffix) { $Version } else { "$Version+main.$Short" }

$Overlay = 'src-tauri/tauri.devinstall.conf.json'
# The overlay is generated by node rather than ConvertTo-Json: PowerShell 5.1
# reorders keys, caps depth and re-escapes content, which would corrupt the CSP
# and the other nested config (same reason release-win-remote.ps1 does a literal
# text swap instead).
#
# The windows array is carried over WHOLESALE from tauri.windows.conf.json with
# only 'title' swapped. Tauri's config merge is recursive for objects but
# REPLACES arrays, so an overlay that names 'app.windows' at all discards every
# key it does not restate, which is how the OS title bar came back on the Linux
# side once ('decorations: false' lost). The assert below fails the build if
# that key ever moves rather than silently shipping the wrong chrome.
$OverlayJs = Join-Path $CacheDir 'overlay.js'
Set-Content -Path $OverlayJs -Encoding ASCII -Value @'
const fs = require("fs");
const conf = JSON.parse(fs.readFileSync("src-tauri/tauri.windows.conf.json", "utf8"));
const windows = conf.app && conf.app.windows;
if (!Array.isArray(windows) || !windows.length) {
  throw new Error("tauri.windows.conf.json has no app.windows to carry over");
}
const w = windows[0];
if (w.decorations !== false) {
  throw new Error("expected decorations:false - did the window config move?");
}
fs.writeFileSync("src-tauri/tauri.devinstall.conf.json", JSON.stringify({
  $schema: "https://schema.tauri.app/config/2",
  productName: process.env.APP_NAME,
  identifier: process.env.APP_ID,
  version: process.env.BUILD_VERSION,
  app: { windows: [{ ...w, title: process.env.APP_NAME }, ...windows.slice(1)] },
  bundle: { createUpdaterArtifacts: false },
}, null, 2) + "\n");
'@

$env:APP_NAME = $AppName
$env:APP_ID = $AppId
$env:BUILD_VERSION = $BuildVersion
Run ('node "' + $OverlayJs + '"')

# --- 4. build ------------------------------------------------------
Step 'pnpm install (frozen lockfile)'
Run 'pnpm install --frozen-lockfile'

# tauri-winres shells out to the Windows SDK resource compiler (rc.exe) to embed
# the app icon/version. Over a bare SSH session the SDK bin is not always on
# PATH, so the build script panics with "have RC.EXE in your $PATH ... or $RC is
# set?". Pin $RC to the newest x64 rc.exe so the build works regardless of PATH.
if (-not $env:RC) {
  $rc = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Recurse -Filter rc.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -like '*\x64\*' } |
    Sort-Object FullName -Descending | Select-Object -First 1
  if ($rc) { $env:RC = $rc.FullName; Step ('Pinned RC=' + $env:RC) }
  else { Warn 'rc.exe not found under Windows Kits; winres may fail' }
}

Step "Building $AppName $BuildVersion (release profile, no bundler)"
# A dev install must never reach for the release updater key, even if this shell
# has it exported from a release run.
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
Info "target dir: $TargetDir - first run is a cold build, later runs are incremental"
$env:CARGO_TARGET_DIR = $TargetDir
Run ('pnpm tauri build --no-bundle --config "' + $Overlay + '"')

# The overlay leaves mainBinaryName alone, so cargo's own bin name is what lands
# here; the install step is what renames it to Hyprlayer-Dev.exe.
$BuiltExe = Join-Path $TargetDir 'release\Hyprlayer.exe'
if (-not (Test-Path $BuiltExe)) { throw "expected binary not found: $BuiltExe" }

# --- 5. install under %LOCALAPPDATA%\Programs ----------------------
Step "Installing to $InstallDir"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# Windows refuses to overwrite or delete a running exe, but it does allow
# renaming one: the live process keeps its open handle while the directory entry
# moves. So park the old exe aside, drop the new one in, then try to delete the
# parked copy (which fails harmlessly while the dev app is still open, and gets
# cleaned up on the next run).
$Staged = "$InstalledExe.new"
$Parked = "$InstalledExe.old"
Remove-Item $Parked -Force -ErrorAction SilentlyContinue
Copy-Item -Force $BuiltExe $Staged
if (Test-Path $InstalledExe) { Move-Item -Force $InstalledExe $Parked }
Move-Item -Force $Staged $InstalledExe
Remove-Item $Parked -Force -ErrorAction SilentlyContinue
if (Test-Path $Parked) { Warn "$AppName is running; the old exe is parked at $Parked and clears on the next run" }

if (AddToUserPath $InstallDir) {
  Info "added $InstallDir to the user PATH"
}
# Make the command work in THIS shell too, not just newly spawned ones.
if (($env:Path -split ';') -notcontains $InstallDir) { $env:Path = "$env:Path;$InstallDir" }

New-Item -ItemType Directory -Force -Path $StartMenu | Out-Null
# The shortcut icon is the one tauri-winres already embedded in the exe, so
# there is nothing to copy alongside it.
$ws = New-Object -ComObject WScript.Shell
$lnk = $ws.CreateShortcut($Shortcut)
$lnk.TargetPath = $InstalledExe
$lnk.WorkingDirectory = $InstallDir
$lnk.IconLocation = "$InstalledExe,0"
$lnk.Description = 'Hyprlayer built from origin/main - testing build, not the release'
$lnk.Save()

Set-Content -Path $Stamp -Value $Sha -Encoding ASCII

Step "Installed $AppName $BuildVersion"
Info "commit    $Short - $Subject"
Info "path      $InstalledExe"
Info "launch    $BinName   (or `"$AppName`" in the Start menu)"
Info "data      $DataDir  (separate from the released app)"
Info "refresh   $Repo\scripts\dev-install-windows.ps1"
