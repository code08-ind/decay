<#
  Checks Decay's Windows installer (P3.4) on a clean hosted Windows machine:
  a silent per-user install puts Decay and decay-host.exe in %LOCALAPPDATA%\Decay and
  registers decay:// and the uninstaller; the app, once started, registers the native
  host with Chrome; the host refuses an unknown extension; installing again while a
  decay-host.exe runs (a browser holds one per connection) still replaces it; a silent
  uninstall removes the files, the host's registry entries and the login item but keeps
  the user's data. Run by .github/workflows/windows-installer-check.yml (on demand).
  Results go to -Out\results.txt; exit code 1 when a check failed.
#>
param(
  [Parameter(Mandatory)] [string] $Installer,
  [Parameter(Mandatory)] [string] $Out
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Installer = (Resolve-Path $Installer).Path
$results = Join-Path $Out 'results.txt'
$failed = 0

function Check([string] $name, [bool] $ok, [string] $detail = '') {
  $line = "{0}  {1}{2}" -f ($(if ($ok) { 'PASS' } else { 'FAIL' })), $name, $(if ($detail) { " ($detail)" } else { '' })
  Write-Host $line
  Add-Content -Path $results -Value $line
  if (-not $ok) { $script:failed++ }
}

function WaitFor([scriptblock] $cond, [int] $seconds) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    if (& $cond) { return $true }
    Start-Sleep -Milliseconds 500
  }
  return [bool](& $cond)
}

$dir = Join-Path $env:LOCALAPPDATA 'Decay'
$host_exe = Join-Path $dir 'decay-host.exe'
$uninstKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Decay'
$chromeHost = 'HKCU:\Software\Google\Chrome\NativeMessagingHosts\com.decay.host'
$support = Join-Path $env:APPDATA 'Decay'

function Install {
  $p = Start-Process -FilePath $Installer -ArgumentList '/S' -Wait -PassThru
  return $p.ExitCode
}
function AppExe {
  Get-ChildItem -Path $dir -Filter *.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notin @('decay-host.exe', 'uninstall.exe') } |
    Select-Object -First 1
}

# --- Install -----------------------------------------------------------------------
$size = (Get-Item $Installer).Length / 1MB
Check 'installer under 20 MB (PRD)' ($size -lt 20) ('{0:N1} MB' -f $size)
$code = Install
Check 'installer exits 0' ($code -eq 0) "exit $code"
$app = AppExe
Check 'app installed per user' ($null -ne $app) $dir
Check 'decay-host.exe next to the app' (Test-Path $host_exe)
Check 'uninstaller registered' (Test-Path $uninstKey)
Check 'decay:// registered' (Test-Path 'HKCU:\Software\Classes\decay')
$sig = Get-AuthenticodeSignature $app.FullName
Add-Content -Path $results -Value ("INFO  app signature: {0}" -f $sig.Status)

# --- The host ----------------------------------------------------------------------
$empty = Join-Path $Out 'empty.txt'
Set-Content -Path $empty -Value $null
$p = Start-Process -FilePath $host_exe -ArgumentList 'chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/' `
  -Wait -PassThru -NoNewWindow -RedirectStandardInput $empty -RedirectStandardOutput (Join-Path $Out 'host-out.bin')
Check 'host refuses an unknown extension' ($p.ExitCode -eq 1) "exit $($p.ExitCode)"

# --- First start: the app registers the host with Chrome ---------------------------
# Until the store IDs are built in, a host needs an allowed origin in host-config.json
# (the app refuses to register one that would accept no extension).
$origin = 'chrome-extension://abcdefghijklmnopabcdefghijklmnop/'
New-Item -ItemType Directory -Force -Path (Join-Path $support 'host') | Out-Null
# (UTF-8 without a BOM: the host's JSON reader refuses one.)
[IO.File]::WriteAllText((Join-Path $support 'host\host-config.json'), (@{ allowedOrigins = @($origin) } | ConvertTo-Json))
# A browser counts as present when its HKCU key exists.
# (Hosted runners have Chrome, whose key can't be re-created: only add it when missing.)
if (-not (Test-Path 'HKCU:\Software\Google\Chrome')) { New-Item -Path 'HKCU:\Software\Google\Chrome' -Force | Out-Null }
$run = Start-Process -FilePath $app.FullName -ArgumentList '--background' -PassThru
$registered = WaitFor { Test-Path $chromeHost } 60
Check 'app registers the host with Chrome' $registered
if ($registered) {
  $manifest = (Get-Item $chromeHost).GetValue('')
  $json = Get-Content -Raw $manifest | ConvertFrom-Json
  Check 'manifest points at the installed host' ($json.path -ieq $host_exe) $json.path
}
Start-Sleep -Seconds 3
Check 'app still running' (-not $run.HasExited)
Stop-Process -Id $run.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

# --- Reinstall while a decay-host.exe runs (a running exe can't be replaced) -------
# An allowed origin: the host then waits for the extension's first message on stdin.
$psi = New-Object System.Diagnostics.ProcessStartInfo $host_exe, $origin
$psi.UseShellExecute = $false
$psi.RedirectStandardInput = $true
$psi.RedirectStandardOutput = $true
$busy = [System.Diagnostics.Process]::Start($psi)
Start-Sleep -Seconds 2
Check 'a connected host keeps running' (-not $busy.HasExited)
$code = Install
Check 'reinstall over a running host exits 0' ($code -eq 0) "exit $code"
Check 'the running host was ended' ($busy.HasExited)
Check 'decay-host.exe present after reinstall' (Test-Path $host_exe)

# --- Uninstall ---------------------------------------------------------------------
$uninstaller = Join-Path $dir 'uninstall.exe'
Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait | Out-Null
# The NSIS uninstaller runs from a copy in %TEMP% and may still be working.
$gone = WaitFor { -not (Test-Path $uninstaller) -and -not (Test-Path $host_exe) } 60
Check 'files removed' $gone
Check 'uninstaller unregistered' (-not (Test-Path $uninstKey))
Check 'host unregistered from Chrome' (-not (Test-Path $chromeHost))
$runKey = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue
Check 'no login item left' ($null -eq $runKey -or $null -eq $runKey.PSObject.Properties['Decay'])
Check "user's data kept (no 'delete data' tick)" (Test-Path $support)

Add-Content -Path $results -Value ("{0} failed" -f $failed)
if ($failed -gt 0) { exit 1 }
