# Keep this BOM-less script ASCII for PowerShell 5.1.
# One phase of scripts/builders/smoke/windows.sh, run on the Windows host.
param(
  [Parameter(Mandatory = $true)][string]$Phase,
  [string]$Setup = '',
  [string]$ServerSha = ''
)
$ErrorActionPreference = 'Stop'
$Work      = Join-Path $env:LOCALAPPDATA 'Temp\hl-smoke'
# Mid-run this holds the only copy of the profile, so it stays out of %TEMP%, which Storage Sense empties.
$Backup    = Join-Path $env:LOCALAPPDATA 'hl-smoke-backup'
$InstDir   = Join-Path $env:LOCALAPPDATA 'Hyprlayer'
$AppExe    = Join-Path $InstDir 'Hyprlayer.exe'
$UninstKey = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Hyprlayer'
$UninstPs  = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Hyprlayer'
$Shortcuts = @{
  'start.lnk'   = (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Hyprlayer.lnk')
  'desktop.lnk' = (Join-Path $env:USERPROFILE 'Desktop\Hyprlayer.lnk')
}
# The release app's profile: database, settings and hosts.json, then WebView2 storage and logs.
$Profiles  = @{
  'profile-roaming' = (Join-Path $env:APPDATA 'ai.brightblock.hyprlayer')
  'profile-local'   = (Join-Path $env:LOCALAPPDATA 'ai.brightblock.hyprlayer')
}

function Check([bool]$ok, [string]$what) {
  if ($ok) { Write-Host ("PASS " + $what) } else { Write-Host ("FAIL " + $what); $script:failed = $true }
}
function Sums([string]$dir) {
  Get-ChildItem $dir -File | Sort-Object Name | ForEach-Object { $_.Name + ' ' + (Get-FileHash $_.FullName).Hash }
}
function Manifest([string]$dir) {
  Get-ChildItem $dir -Recurse -File -Force | ForEach-Object { $_.FullName.Substring($dir.Length) + ' ' + $_.Length + ' ' + $_.LastWriteTimeUtc.Ticks } | Sort-Object
}
# An SSH session cannot reach the desktop, so GUI steps run as a task in the
# logged-on user's session.
function Interactive([string]$Exe, [string]$Arguments) {
  $name = 'hl-smoke-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
  $user = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
  $action = New-ScheduledTaskAction -Execute $Exe -Argument $Arguments
  $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
  Register-ScheduledTask -TaskName $name -Action $action -Principal $principal | Out-Null
  Start-ScheduledTask -TaskName $name
}
# reg.exe reports success on stderr, which 5.1 turns into a terminating error under Stop.
function Reg([string]$CmdLine) {
  cmd /c ('reg ' + $CmdLine + ' >nul 2>&1')
  if ($LASTEXITCODE -ne 0) { throw ('reg failed: ' + $CmdLine) }
}
function Wait-Until([scriptblock]$Cond, [int]$Seconds) {
  for ($i = 0; $i -lt $Seconds; $i++) { if (& $Cond) { return $true }; Start-Sleep -Seconds 1 }
  return $false
}
# WebView2 can hold its profile for a few seconds after the app exits.
function Remove-Tree([string]$Path) {
  for ($i = 0; $i -lt 30; $i++) {
    try { Remove-Item $Path -Recurse -Force -ErrorAction Stop; return } catch { Start-Sleep -Seconds 1 }
  }
  Remove-Item $Path -Recurse -Force
}
$script:failed = $false

switch ($Phase) {
  'backup' {
    if (Test-Path $Backup) { throw ("a previous run left " + $Backup + ", which may hold the Hyprlayer install and profile; restore or remove it by hand") }
    # Installing closes the app and uninstalling kills the server, and with it any agents it hosts.
    $live = Get-Process -Name Hyprlayer, hyprlayer-server -ErrorAction SilentlyContinue
    if ($live) { throw ("Hyprlayer is in use on this host (" + (($live | ForEach-Object { $_.Name }) -join ', ') + "); quit it and its server first") }
    New-Item -ItemType Directory -Force -Path (Join-Path $Backup 'files') | Out-Null
    if (Test-Path $InstDir) {
      Copy-Item (Join-Path $InstDir '*') (Join-Path $Backup 'files') -Force
      Reg ('export "' + $UninstKey + '" "' + (Join-Path $Backup 'uninstall.reg') + '" /y')
      foreach ($k in $Shortcuts.Keys) { if (Test-Path $Shortcuts[$k]) { Copy-Item $Shortcuts[$k] (Join-Path $Backup $k) } }
      Write-Host ("backed up " + (Get-ChildItem $InstDir -File).Count + " files from " + $InstDir)
    } else {
      New-Item (Join-Path $Backup 'none') -ItemType File | Out-Null
      Write-Host "no existing install"
    }
    # Moved, never copied: a server opened on a copied database sweeps the real worktrees.
    # Directory.Move renames in one step; Move-Item falls back to moving file by file,
    # so one locked file would leave a profile split between here and the backup.
    $moved = @()
    try {
      foreach ($k in $Profiles.Keys) {
        if (Test-Path $Profiles[$k]) {
          [System.IO.File]::WriteAllLines((Join-Path $Backup ($k + '.manifest')), [string[]]@(Manifest $Profiles[$k]))
          [System.IO.Directory]::Move($Profiles[$k], (Join-Path $Backup $k))
          $moved += $k
        } else {
          New-Item (Join-Path $Backup ($k + '.absent')) -ItemType File | Out-Null
        }
      }
    } catch {
      foreach ($k in $moved) { [System.IO.Directory]::Move((Join-Path $Backup $k), $Profiles[$k]) }
      # The rest of the backup is copies, but never delete one that still holds a profile.
      if (-not ($Profiles.Keys | Where-Object { Test-Path (Join-Path $Backup $_) })) { Remove-Item $Backup -Recurse -Force }
      throw
    }
    Write-Host ("moved " + $moved.Count + " profile dirs aside")
  }
  'install' {
    $p = Start-Process -FilePath $Setup -ArgumentList '/S' -Wait -PassThru
    Check ($p.ExitCode -eq 0) ("silent install exits 0 (got " + $p.ExitCode + ")")
    Check (Test-Path $AppExe) "Hyprlayer.exe installed"
    $server = Join-Path $InstDir 'hyprlayer-server.exe'
    Check ((Get-FileHash $server).Hash -eq $ServerSha.ToUpper()) "installed server matches the release asset"
    Write-Host ("server --version: " + (& $server --version))
    $reg = Get-ItemProperty $UninstPs -ErrorAction SilentlyContinue
    Check ($reg.DisplayName -eq 'Hyprlayer Desktop') "POSTINSTALL hook retitled the uninstall entry"
    Write-Host ("DisplayVersion: " + $reg.DisplayVersion + "  FileVersion: " + (Get-Item $AppExe).VersionInfo.FileVersion)
    foreach ($k in $Shortcuts.Keys) { Check (Test-Path $Shortcuts[$k]) ($k + " created") }
  }
  'launch' {
    Interactive 'explorer.exe' ('"' + $AppExe + '"')
    $up = Wait-Until { Get-Process -Name Hyprlayer -ErrorAction SilentlyContinue } 30
    Check $up "Hyprlayer.exe running in the user's session"
    Start-Sleep -Seconds 20
    $out = Join-Path $Work 'probe'
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    Interactive 'powershell.exe' ('-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "' + (Join-Path $Work 'windows-host.ps1') + '" -Phase probe')
    Check (Wait-Until { Test-Path (Join-Path $out 'done') } 30) "window probe finished"
    $window = Get-Content (Join-Path $out 'window.txt')
    Write-Host ("window: " + $window)
    Check ($window -match "title='Hyprlayer' responding=True") "main window is up and responding"
    $kids = (Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -in (Get-Process -Name Hyprlayer).Id } | ForEach-Object { $_.Name }) -join ','
    Write-Host ("children: " + $kids)
    Check ($kids -match 'msedgewebview2') "WebView2 started"
  }
  'probe' {
    # Runs inside the user's session; window titles and the screen are only visible there.
    $out = Join-Path $Work 'probe'
    $p = Get-Process -Name Hyprlayer -ErrorAction SilentlyContinue | Select-Object -First 1
    ("title='" + $p.MainWindowTitle + "' responding=" + $p.Responding) | Set-Content (Join-Path $out 'window.txt')
    try {
      Add-Type -AssemblyName System.Windows.Forms, System.Drawing
      $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
      $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
      [System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
      $bmp.Save((Join-Path $out 'screenshot.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    } catch { ("screenshot failed: " + $_) | Add-Content (Join-Path $out 'window.txt') }
    'done' | Set-Content (Join-Path $out 'done')
  }
  'close' {
    Interactive 'taskkill.exe' '/IM Hyprlayer.exe'
    if (Wait-Until { -not (Get-Process -Name Hyprlayer -ErrorAction SilentlyContinue) } 15) {
      Write-Host "app exited on WM_CLOSE"
    } else {
      # The quit confirmation, on by default, keeps the app open after WM_CLOSE.
      Write-Host "app stayed open on WM_CLOSE (quit confirmation); stopping it"
      Stop-Process -Name Hyprlayer -Force -ErrorAction SilentlyContinue
    }
    Check (Wait-Until { -not (Get-Process -Name Hyprlayer -ErrorAction SilentlyContinue) } 15) "app stopped before uninstall"
  }
  'uninstall' {
    Start-Process -FilePath (Join-Path $InstDir 'uninstall.exe') -ArgumentList '/S' | Out-Null
    # NSIS re-launches the uninstaller from TEMP as Un_A.exe, so wait on that copy.
    $done = Wait-Until { -not (Test-Path $UninstPs) -and -not (Get-Process -Name 'Un_A', 'uninstall' -ErrorAction SilentlyContinue) } 90
    Check $done "uninstaller finished"
    Check (-not (Test-Path $InstDir)) "install dir removed"
    foreach ($k in $Shortcuts.Keys) { Check (-not (Test-Path $Shortcuts[$k])) ($k + " removed") }
    Check (Test-Path (Join-Path $env:APPDATA 'ai.brightblock.hyprlayer')) "app data kept"
  }
  'restore' {
    # The profile first: it is the copy nothing else can rebuild. Anything still
    # running from the test install holds files in the test's profile.
    Get-Process -Name Hyprlayer, hyprlayer-server -ErrorAction SilentlyContinue |
      Where-Object { $_.Path -like ($InstDir + '\*') } | Stop-Process -Force -ErrorAction SilentlyContinue
    foreach ($k in $Profiles.Keys) {
      $saved = Join-Path $Backup $k
      # Without the saved copy or the marker, the live dir is the real profile.
      if (-not (Test-Path $saved) -and -not (Test-Path ($saved + '.absent'))) { continue }
      if (Test-Path $Profiles[$k]) { Remove-Tree $Profiles[$k] }
      if (Test-Path $saved) {
        [System.IO.Directory]::Move($saved, $Profiles[$k])
        # A failed comparison must not stop the install restore that follows.
        $same = $false
        try {
          $same = ([System.IO.File]::ReadAllLines($saved + '.manifest') -join "`n") -eq (@(Manifest $Profiles[$k]) -join "`n")
        } catch { Write-Host ("manifest check failed: " + $_) }
        Check $same ("profile back unchanged at " + $Profiles[$k])
      } else {
        Remove-Item ($saved + '.absent')
      }
    }
    if (Test-Path (Join-Path $Backup 'none')) {
      Write-Host "nothing was installed before; leaving it uninstalled"
    } else {
      New-Item -ItemType Directory -Force -Path $InstDir | Out-Null
      Copy-Item (Join-Path $Backup 'files\*') $InstDir -Force
      Reg ('import "' + (Join-Path $Backup 'uninstall.reg') + '"')
      foreach ($k in $Shortcuts.Keys) { if (Test-Path (Join-Path $Backup $k)) { Copy-Item (Join-Path $Backup $k) $Shortcuts[$k] -Force } }
      Check ($null -eq (Compare-Object (Sums (Join-Path $Backup 'files')) (Sums $InstDir))) "previous install restored byte for byte"
      Check (Test-Path $UninstPs) "uninstall entry restored"
    }
  }
  'cleanup' {
    Get-ScheduledTask -TaskName 'hl-smoke-*' -ErrorAction SilentlyContinue | Unregister-ScheduledTask -Confirm:$false
    if (-not ($Profiles.Keys | Where-Object { Test-Path (Join-Path $Backup $_) })) { Remove-Item $Backup -Recurse -Force -ErrorAction SilentlyContinue }
  }
  default { throw ("unknown phase " + $Phase) }
}
if ($script:failed) { exit 1 }
