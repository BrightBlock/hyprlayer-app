#requires -Version 5
# Keep this BOM-less script ASCII for PowerShell 5.1.

[CmdletBinding()]
param(
  [string] $Ref = 'origin/main',
  [switch] $Force,
  [switch] $NoFetch,
  [switch] $NoVersionSuffix,
  [switch] $Uninstall
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if ($env:OS -ne 'Windows_NT') {
  throw 'this is the Windows installer; use scripts/dev-install-macos.sh or scripts/dev-install-linux.sh'
}

$OriginSlug = 'BrightBlock/hyprlayer-app'
$OriginUrl  = if ($env:HYPRLAYER_DEV_ORIGIN) { $env:HYPRLAYER_DEV_ORIGIN } else { "https://github.com/$OriginSlug.git" }
$Bootstrap  = "& ([scriptblock]::Create((gh api repos/$OriginSlug/contents/scripts/dev-install-windows.ps1 -H 'Accept: application/vnd.github.raw' | Out-String)))"

$Repo = ''
if ($PSScriptRoot) {
  $candidate = (Resolve-Path (Join-Path $PSScriptRoot '..')).ProviderPath
  if ((Test-Path (Join-Path $candidate '.git')) -and (Test-Path (Join-Path $candidate 'src-tauri'))) {
    $Repo = $candidate
  }
}

$CacheDir  = if ($env:HYPRLAYER_DEV_CACHE) { $env:HYPRLAYER_DEV_CACHE } else { Join-Path $env:USERPROFILE '.hyprlayer-dev' }
$SrcDir    = Join-Path $CacheDir 'src'
$TargetDir = Join-Path $CacheDir 'target'
$Stamp     = Join-Path $CacheDir 'installed'

$Prefix   = if ($env:HYPRLAYER_DEV_PREFIX) { $env:HYPRLAYER_DEV_PREFIX } else { Join-Path $env:LOCALAPPDATA 'Programs' }
$AppId    = 'ai.brightblock.hyprlayer.main'
$AppName  = 'Hyprlayer Dev'
$BinName  = 'Hyprlayer-Dev'

$InstallDir   = Join-Path $Prefix $BinName
$InstalledExe = Join-Path $InstallDir "$BinName.exe"
# supervisor.rs spawns the server from beside the app binary.
$InstalledServer = Join-Path $InstallDir 'hyprlayer-server.exe'
$StartMenu    = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$Shortcut     = Join-Path $StartMenu "$AppName.lnk"
$DataDir      = Join-Path $env:APPDATA $AppId
$WebviewDir   = Join-Path $env:LOCALAPPDATA $AppId

function Step($m) { Write-Host ('==> ' + $m) -ForegroundColor Cyan }
function Info($m) { Write-Host ('    ' + $m) }
function Warn($m) { Write-Warning $m }

function Run($cmdline) {
  cmd /c $cmdline
  if ($LASTEXITCODE -ne 0) { throw ('command failed (' + $LASTEXITCODE + '): ' + $cmdline) }
}

function RunGit([string[]] $GitArgs, [switch] $Quiet) {
  $ErrorActionPreference = 'Continue'
  $out = & git @GitArgs 2>&1
  if ($LASTEXITCODE -ne 0) {
    if ($Quiet) { return $null }
    throw ('git ' + ($GitArgs -join ' ') + ' failed: ' + (($out | Out-String).Trim()))
  }
  return @($out | Where-Object { $_ -isnot [System.Management.Automation.ErrorRecord] })
}

function GitLine([string[]] $GitArgs) {
  $out = RunGit $GitArgs -Quiet
  if ($null -eq $out) { return $null }
  $lines = @($out)
  if ($lines.Count -eq 0) { return $null }
  return ([string]$lines[0]).Trim()
}

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
  try {
    if (-not ('Win32.EnvBroadcast' -as [type])) {
      Add-Type -Namespace Win32 -Name EnvBroadcast -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Auto)]
public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);
'@
    }
    $result = [System.UIntPtr]::Zero
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

if (-not (Test-Path (Join-Path $SrcDir '.git'))) {
  Step "Creating build clone at $SrcDir"
  New-Item -ItemType Directory -Force -Path $CacheDir | Out-Null
  if (Test-Path $SrcDir) { Remove-Item -Recurse -Force $SrcDir }
  if ($Repo) {
    RunGit @('clone', '--quiet', $Repo, $SrcDir) | Out-Null
    $originUrl = GitLine @('-C', $Repo, 'remote', 'get-url', 'origin')
    if ($originUrl) { RunGit @('-C', $SrcDir, 'remote', 'set-url', 'origin', $originUrl) | Out-Null }
  } elseif ((-not $env:HYPRLAYER_DEV_ORIGIN) -and (Get-Command gh -ErrorAction SilentlyContinue)) {
    Info "source: $OriginSlug via gh"
    try { Run ('gh repo clone ' + $OriginSlug + ' "' + $SrcDir + '" -- --quiet') }
    catch { throw ("clone failed; is gh authenticated for $OriginSlug? (gh auth status): " + $_.Exception.Message) }
  } else {
    Info "source: $OriginUrl"
    RunGit @('clone', '--quiet', $OriginUrl, $SrcDir) | Out-Null
  }
}

if (-not $NoFetch) {
  Step "Fetching $Ref"
  try { RunGit @('-C', $SrcDir, 'fetch', '--quiet', '--prune', '--tags', 'origin') | Out-Null }
  catch { throw ('fetch failed (try -NoFetch to build what is already here): ' + $_.Exception.Message) }
}

$Sha = GitLine @('-C', $SrcDir, 'rev-parse', '--verify', "$Ref^{commit}")
if (-not $Sha) { throw "no such ref in the build clone: $Ref" }
# Fixed width, as in dev-install-linux.sh: the app compares this stamp with a remote host's.
$Short   = $Sha.Substring(0, 8)
$Commits = GitLine @('-C', $SrcDir, 'rev-list', '--count', $Sha)
$Subject = GitLine @('-C', $SrcDir, 'log', '-1', '--format=%s', $Sha)

$StampSha = if (Test-Path $Stamp) { ([string](Get-Content $Stamp -Raw)).Trim() } else { '' }
if ((-not $Force) -and (Test-Path $InstalledExe) -and $StampSha -eq $Sha) {
  Step "Already on $Ref ($Short - $Subject)"
  Info 'Nothing to build. Use -Force to rebuild anyway.'
  exit 0
}

Step "Checking out $Short - $Subject"
# --force: a prior build leaves its +dev version stamp in Cargo.toml/Cargo.lock.
RunGit @('-C', $SrcDir, 'checkout', '--quiet', '--force', '--detach', $Sha) | Out-Null
RunGit @('-C', $SrcDir, 'reset', '--hard', '--quiet', $Sha) | Out-Null
RunGit @('-C', $SrcDir, 'clean', '-qfd') | Out-Null

Set-Location $SrcDir

# package.json only bumps in the release commit, so a build off untagged main would
# report the *previous* release and the updater would offer an upgrade to the one it
# already contains. Take the newest tag instead, and keep the sha as build metadata
# ('+', not '-') so semver reads it as equal to that release rather than older. The
# commit count before the sha is what lets the app order two dev builds.
$Version = (Get-Content (Join-Path $SrcDir 'package.json') -Raw | ConvertFrom-Json).version
$LatestTag = GitLine @('-C', $SrcDir, 'tag', '--list', 'v[0-9]*', '--sort=-v:refname')
if ($LatestTag) {
  $tagVersion = $LatestTag.TrimStart('v')
  if ([version]$tagVersion -gt [version]$Version) { $Version = $tagVersion }
}
$BuildVersion = if ($NoVersionSuffix) { $Version } else { "$Version+dev.$Commits.$Short" }

$Overlay = 'src-tauri/tauri.devinstall.conf.json'
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

// The app reports the workspace version; without this a dev build claims to be
// the release it descends from.
const manifest = fs.readFileSync("Cargo.toml", "utf8");
// Exactly one, not at least one: `replace` without /g rewrites the *first*
// match, so a second `version = "..."` at column 0 would silently send this
// to the wrong line.
const hits = manifest.match(/^version = ".*"$/gm) || [];
if (hits.length !== 1) {
  throw new Error("expected exactly one workspace version line in Cargo.toml, found " + hits.length);
}
fs.writeFileSync(
  "Cargo.toml",
  manifest.replace(/^version = ".*"$/m, 'version = "' + process.env.BUILD_VERSION + '"'),
);

// The server is versioned apart from the app, and its version keys the remote
// install directory: without the same +dev suffix it collides with the release.
const serverToml = "crates/hyprlayer-server/Cargo.toml";
const server = fs.readFileSync(serverToml, "utf8");
const serverHits = server.match(/^version = ".*"$/gm) || [];
if (serverHits.length !== 1) {
  throw new Error("expected exactly one version line in " + serverToml + ", found " + serverHits.length);
}
const plus = process.env.BUILD_VERSION.indexOf("+");
const suffix = plus === -1 ? "" : process.env.BUILD_VERSION.slice(plus);
fs.writeFileSync(
  serverToml,
  server.replace(/^version = "([^"+]*)[^"]*"$/m, (_, base) => 'version = "' + base + suffix + '"'),
);
'@

$env:APP_NAME = $AppName
$env:APP_ID = $AppId
$env:BUILD_VERSION = $BuildVersion
Run ('node "' + $OverlayJs + '"')

Step 'pnpm install (frozen lockfile)'
Run 'pnpm install --frozen-lockfile'

if (-not $env:RC) {
  $rc = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Recurse -Filter rc.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -like '*\x64\*' } |
    Sort-Object FullName -Descending | Select-Object -First 1
  if ($rc) { $env:RC = $rc.FullName; Step ('Pinned RC=' + $env:RC) }
  else { Warn 'rc.exe not found under Windows Kits; winres may fail' }
}

Step "Building $AppName $BuildVersion (release profile, no bundler)"
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
Info "target dir: $TargetDir - first run is a cold build, later runs are incremental"
$env:CARGO_TARGET_DIR = $TargetDir
Run ('pnpm tauri build --no-bundle --config "' + $Overlay + '"')

$BuiltExe = Join-Path $TargetDir 'release\Hyprlayer.exe'
if (-not (Test-Path $BuiltExe)) { throw "expected binary not found: $BuiltExe" }
# --no-bundle does no bundling, so nothing installs the sidecar; without this the
# app starts, finds no server beside it, and dies in the ENOENT dialog.
$BuiltServer = Join-Path $TargetDir 'release\hyprlayer-server.exe'
if (-not (Test-Path $BuiltServer)) {
  throw "expected server not found: $BuiltServer - did beforeBuildCommand run?"
}

Step "Installing to $InstallDir"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$Staged = "$InstalledExe.new"
$Parked = "$InstalledExe.old"
Remove-Item $Parked -Force -ErrorAction SilentlyContinue
Copy-Item -Force $BuiltExe $Staged
if (Test-Path $InstalledExe) { Move-Item -Force $InstalledExe $Parked }
Move-Item -Force $Staged $InstalledExe
Remove-Item $Parked -Force -ErrorAction SilentlyContinue
if (Test-Path $Parked) { Warn "$AppName is running; the old exe is parked at $Parked and clears on the next run" }

# The server is a deliberate orphan and outlives the app, so it is more likely
# than the app to be running here. Same park-and-swap.
$ServerStaged = "$InstalledServer.new"
$ServerParked = "$InstalledServer.old"
Remove-Item $ServerParked -Force -ErrorAction SilentlyContinue
Copy-Item -Force $BuiltServer $ServerStaged
if (Test-Path $InstalledServer) { Move-Item -Force $InstalledServer $ServerParked }
Move-Item -Force $ServerStaged $InstalledServer
Remove-Item $ServerParked -Force -ErrorAction SilentlyContinue
if (Test-Path $ServerParked) {
  Warn "a hyprlayer-server is still running; the old exe is parked at $ServerParked - run ``hyprlayer-server stop`` and rerun"
}

if (AddToUserPath $InstallDir) {
  Info "added $InstallDir to the user PATH"
}
if (($env:Path -split ';') -notcontains $InstallDir) { $env:Path = "$env:Path;$InstallDir" }

New-Item -ItemType Directory -Force -Path $StartMenu | Out-Null
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
if ($Repo) { Info "refresh   $Repo\scripts\dev-install-windows.ps1" }
else { Info ('refresh   ' + $Bootstrap) }
