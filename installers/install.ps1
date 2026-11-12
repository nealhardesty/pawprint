# PAWPRINT installer for Windows (SPEC section 15.5). Windows PowerShell 5.1 and PowerShell 7.
#
#   irm https://github.com/nealhardesty/pawprint/releases/latest/download/install.ps1 | iex
#   & ([scriptblock]::Create((irm https://github.com/nealhardesty/pawprint/releases/latest/download/install.ps1))) -Version 1.2.0 -NoAutostart
#
# Options: -Version V, -Latest (print the latest release's version and change
# nothing), -Archive FILE (offline; FILE.sha256 next to it is checked when it
# is there), -Prefix DIR, -NoAutostart, -NoStart, -Uninstall, -Purge (with
# -Uninstall: also delete config, state, and logs), -Help.
# Environment: PAWPRINT_VERSION, PAWPRINT_PREFIX, and PAWPRINT_DOWNLOAD_BASE
# (replaces https://github.com/nealhardesty/pawprint/releases; a mirror laid
# out the same way).
#
# Nothing changes until the download is verified and the new binary has run.
# It never calls `exit`, so running it through `iex` can't close the window.

param(
    [string]$Version = $env:PAWPRINT_VERSION,
    [switch]$Latest,
    [string]$Archive,
    [string]$Prefix = $env:PAWPRINT_PREFIX,
    [switch]$NoAutostart,
    [switch]$NoStart,
    [switch]$Uninstall,
    [switch]$Purge,
    [switch]$Help
)

function Invoke-PawprintInstaller {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'  # 5.1's progress bar slows downloads to a crawl

    if ($Help) {
        Write-Host @'
Install PAWPRINT, the pixel-art hostname badge, for this user.

  -Version V     install version V (default: the latest release)
  -Latest        print the latest release's version and change nothing
  -Archive FILE  install from a downloaded .zip, with no network access
                 (FILE.sha256 next to it is checked when it is there)
  -Prefix DIR    install into DIR (default: %LOCALAPPDATA%\Programs\pawprint)
  -NoAutostart   don't start PAWPRINT at login
  -NoStart       don't start the badge now
  -Uninstall     remove PAWPRINT and its autostart entry (your config is kept)
  -Purge         with -Uninstall, also delete your config, state, and logs

Environment: PAWPRINT_VERSION and PAWPRINT_PREFIX set the defaults for
-Version and -Prefix. PAWPRINT_DOWNLOAD_BASE replaces
https://github.com/nealhardesty/pawprint/releases (a mirror laid out the same
way: download/v<version>/<asset>, with latest redirecting to tag/v<version>).
'@
        return
    }
    if ($Purge -and -not $Uninstall) { throw 'pawprint: -Purge goes with -Uninstall' }

    $prefixDir = if ($Prefix) { $Prefix } else { Join-Path $env:LOCALAPPDATA 'Programs\pawprint' }
    # Relative to this shell's location ($PWD, not the process directory that
    # GetFullPath uses), ~ expanded, and with no trailing separator except
    # on a drive root.
    $prefixDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($prefixDir)
    if ($prefixDir.Length -gt 3) { $prefixDir = $prefixDir.TrimEnd('\', '/') }
    $exe = Join-Path $prefixDir 'pawprint.exe'
    $base = if ($env:PAWPRINT_DOWNLOAD_BASE) { $env:PAWPRINT_DOWNLOAD_BASE } else { 'https://github.com/nealhardesty/pawprint/releases' }
    $names = 'pawprint.exe', 'pawprintw.exe'

    if ($Uninstall) {
        Remove-Pawprint $prefixDir $exe $names
        return
    }

    if ($env:PROCESSOR_ARCHITECTURE -eq 'x86' -and -not $env:PROCESSOR_ARCHITEW6432) {
        throw 'pawprint: 32-bit Windows is not supported'
    }
    $target = 'x86_64-pc-windows-msvc'  # also runs on Windows on Arm, under emulation
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    if ($Latest) {
        Write-Host (Get-PawprintLatest $base)
        return
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("pawprint-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        if ($Archive) {
            if (-not (Test-Path -LiteralPath $Archive -PathType Leaf)) { throw "pawprint: no such archive: $Archive" }
            $zip = (Resolve-Path -LiteralPath $Archive).Path
            $sums = "$zip.sha256"
            if (-not (Test-Path -LiteralPath $sums -PathType Leaf)) { $sums = $null }
        } else {
            $ver = if ($Version) { $Version -replace '^v', '' } else { Get-PawprintLatest $base }
            $name = "pawprint-$ver-$target.zip"
            $url = "$base/download/v$ver/$name"
            Write-Host "Downloading PAWPRINT $ver for $target"
            $zip = Join-Path $tmp $name
            $sums = "$zip.sha256"
            foreach ($pair in @(@($url, $zip), @("$url.sha256", $sums))) {
                try { Invoke-WebRequest -Uri $pair[0] -OutFile $pair[1] -UseBasicParsing }
                catch { throw "pawprint: download failed: $($pair[0]) ($($_.Exception.Message))" }
            }
        }

        if ($sums) {
            $want = ((Get-Content -LiteralPath $sums -TotalCount 1) -split '\s+')[0].ToLowerInvariant()
            $got = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($got -ne $want) { throw "pawprint: checksum mismatch for $zip (expected $want, got $got); nothing was changed" }
            Write-Host "Checksum OK ($got)"
        } else {
            Write-Warning "no $zip.sha256 next to the archive, so its checksum is not checked"
        }

        $x = Join-Path $tmp 'x'
        Expand-Archive -LiteralPath $zip -DestinationPath $x
        foreach ($n in $names) {
            if (-not (Test-Path -LiteralPath (Join-Path $x $n))) { throw "pawprint: $zip has no $n in it" }
        }
        Get-ChildItem -LiteralPath $x | Unblock-File
        $new = Join-Path $x 'pawprint.exe'
        $r = Invoke-Native $new @('--version')
        if ($r.Code -ne 0) { throw "pawprint: the new binary does not run on this machine: $($r.Text)" }
        $newVersion = $r.Text

        # Verified. From here on the system changes.
        New-Item -ItemType Directory -Force -Path $prefixDir | Out-Null
        if (Test-Path -LiteralPath $exe) {
            $old = (Invoke-Native $exe @('--version')).Text
            Write-Host "Upgrading $old to $newVersion"
            $was = Stop-Badge $exe
        } else {
            $was = Stop-Badge $new
        }
        foreach ($n in $names) {
            $dst = Join-Path $prefixDir $n
            # A running or just-exited exe can be renamed but not overwritten.
            Remove-Item -LiteralPath "$dst.old" -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $dst) { Move-Item -LiteralPath $dst -Destination "$dst.old" -Force }
            Copy-Item -LiteralPath (Join-Path $x $n) -Destination $dst
            Remove-Item -LiteralPath "$dst.old" -Force -ErrorAction SilentlyContinue
        }
        Write-Host "Installed $newVersion to $prefixDir"
        if (Add-UserPath $prefixDir) {
            Write-Host "Added $prefixDir to your user PATH (new terminals pick it up)"
        }
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    if (-not $NoAutostart) {
        $r = Invoke-Native $exe @('autostart', 'on')
        if ($r.Code -eq 0) { Write-Host $r.Text } else { Write-Warning "could not turn on autostart: $($r.Text)" }
    }
    if (-not $NoStart) {
        # Start-Process goes through ShellExecute, which passes no handles
        # down. Called directly, the long-lived badge would inherit this
        # shell's stdout and hold open the pipe of anything capturing it.
        $p = Start-Process -FilePath $exe -WindowStyle Hidden -PassThru
        $null = $p.Handle  # 5.1 loses ExitCode unless the handle is opened early
        $p.WaitForExit()
        if ($p.ExitCode -eq 0) { Write-Host 'PAWPRINT is running.' }
        else { Write-Warning "the badge did not start (exit code $($p.ExitCode)); ``pawprint doctor`` explains why" }
    } elseif ($was) {
        Write-Host "The badge was stopped for the upgrade; start it again with: pawprint"
    }
    Write-Host ''
    & $exe explain | Out-Host
}

# Run a native command; stdout and stderr together as text, plus the exit code.
function Invoke-Native([string]$Path, [string[]]$Arguments) {
    $ErrorActionPreference = 'Continue'  # 5.1 turns redirected stderr into errors under 'Stop'
    $out = & $Path @Arguments 2>&1 | ForEach-Object { "$_" }
    @{ Code = $LASTEXITCODE; Text = (($out | Where-Object { $_ }) -join "`n").Trim() }
}

# GitHub redirects releases/latest to releases/tag/v<version>.
function Get-PawprintLatest([string]$Base) {
    try {
        $req = [Net.WebRequest]::Create("$Base/latest")
        $req.Method = 'HEAD'
        $resp = $req.GetResponse()
        $uri = $resp.ResponseUri.AbsoluteUri
        $resp.Close()
    } catch { throw "pawprint: cannot reach $Base/latest ($($_.Exception.Message)); pass -Version" }
    if ($uri -match '/tag/v?([^/]+)/?$') { return $Matches[1] }
    throw "pawprint: cannot find the latest version at $Base/latest; pass -Version"
}

# Stop the badge that $Exe talks to, if one is running; $true if it was.
function Stop-Badge([string]$Exe) {
    if ((Invoke-Native $Exe @('quit')).Code -ne 0) { return $false }
    for ($i = 0; $i -lt 20 -and (Invoke-Native $Exe @('status')).Code -eq 0; $i++) { Start-Sleep -Milliseconds 250 }
    Start-Sleep -Milliseconds 250  # let the process exit after its pipe closes
    return $true
}

# The user PATH from the registry, keeping %VARIABLES% unexpanded.
function Get-UserPathEntries {
    $key = Get-Item -LiteralPath 'HKCU:\Environment'
    $raw = $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    @($raw -split ';' | Where-Object { $_ })
}

function Set-UserPathEntries([string[]]$Entries) {
    Set-ItemProperty -LiteralPath 'HKCU:\Environment' -Name Path -Value ($Entries -join ';') -Type ExpandString
    # Setting any user variable broadcasts WM_SETTINGCHANGE, so Explorer and
    # new terminals see the new PATH.
    [Environment]::SetEnvironmentVariable('PAWPRINT_INSTALLER', $null, 'User')
}

function Add-UserPath([string]$Dir) {
    if (($env:Path -split ';') -notcontains $Dir) { $env:Path = "$env:Path;$Dir" }
    $entries = Get-UserPathEntries
    if ($entries -contains $Dir) { return $false }
    Set-UserPathEntries ($entries + $Dir)
    return $true
}

function Remove-Pawprint([string]$PrefixDir, [string]$Exe, [string[]]$Names) {
    if (Test-Path -LiteralPath $Exe) {
        $r = Invoke-Native $Exe @('autostart', 'off')
        if ($r.Code -ne 0) { Write-Warning "pawprint autostart off failed; removing the entry directly: $($r.Text)" }
        Stop-Badge $Exe | Out-Null
        foreach ($n in $Names) {
            Remove-Item -LiteralPath (Join-Path $PrefixDir $n), (Join-Path $PrefixDir "$n.old") -Force -ErrorAction SilentlyContinue
        }
        if (Test-Path -LiteralPath $Exe) { throw "pawprint: cannot remove $Exe" }
        Write-Host "Removed PAWPRINT from $PrefixDir"
        if (-not (Get-ChildItem -LiteralPath $PrefixDir -Force)) { Remove-Item -LiteralPath $PrefixDir -Force }
    } else {
        Write-Host "PAWPRINT is not installed in $PrefixDir"
    }
    $run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    if ((Get-Item -LiteralPath $run).GetValue('pawprint')) {
        Remove-ItemProperty -LiteralPath $run -Name 'pawprint'
        Write-Host "Removed the autostart entry"
    }
    $entries = Get-UserPathEntries
    if ($entries -contains $PrefixDir) {
        Set-UserPathEntries @($entries | Where-Object { $_ -ne $PrefixDir })
        Write-Host "Removed $PrefixDir from your user PATH"
    }
    $configDir = Join-Path $env:APPDATA 'pawprint'
    if ($Purge) {
        foreach ($dir in @($configDir, (Join-Path $env:LOCALAPPDATA 'pawprint'))) {
            if (Test-Path -LiteralPath $dir) {
                Remove-Item -LiteralPath $dir -Recurse -Force
                Write-Host "Deleted $dir"
            }
        }
    } else {
        Write-Host "Your config and custom mascots are kept in $configDir (-Purge deletes them)."
    }
}

Invoke-PawprintInstaller
