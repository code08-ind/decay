<#
  Drives a debug build of Decay on a Windows desktop and checks the P3.3 chrome:
  the frameless windows move, resize, maximise and Snap; light and dark themes; the
  search panel; the tray popover's placement and click-to-close. Screenshots and
  results.txt go to -Out. Run by .github/workflows/windows-ui-check.yml (on demand).
  Exit code 1 when a check failed (the screenshots are uploaded either way).
#>
param(
  [Parameter(Mandatory)] [string] $Exe,
  [Parameter(Mandatory)] [string] $Out
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Exe = (Resolve-Path $Exe).Path

Add-Type -AssemblyName System.Drawing, System.Windows.Forms, UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int size);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int x, int y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint f, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  public static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, string l, uint flags, uint timeout, out IntPtr result);

  public static RECT Bounds(IntPtr h) {
    RECT r;
    // DWMWA_EXTENDED_FRAME_BOUNDS: the visible frame, without the invisible resize border.
    if (DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) != 0) GetWindowRect(h, out r);
    return r;
  }

  public static List<IntPtr> Visible(uint pid) {
    var list = new List<IntPtr>();
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) list.Add(h);
      return true;
    }, IntPtr.Zero);
    return list;
  }

  [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int a, out int v, int size);
  /// An int DWM attribute (38 backdrop type, 33 corner preference); -1 when unreadable.
  public static int DwmInt(IntPtr h, int a) {
    int v; return DwmGetWindowAttribute(h, a, out v, 4) == 0 ? v : -1;
  }
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);

  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmDeviceName;
    public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra;
    public int dmFields, dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
    public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmFormName;
    public short dmLogPixels;
    public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
    public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight;
  }
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool EnumDisplaySettings(string d, int mode, ref DEVMODE m);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int ChangeDisplaySettings(ref DEVMODE m, int flags);
  public static int SetResolution(int w, int h) {
    var m = new DEVMODE(); m.dmSize = (short)Marshal.SizeOf(typeof(DEVMODE));
    if (!EnumDisplaySettings(null, -1, ref m)) return -100;
    m.dmPelsWidth = w; m.dmPelsHeight = h; m.dmFields = 0x80000 | 0x100000;
    return ChangeDisplaySettings(ref m, 0);
  }

  public static string Title(IntPtr h) {
    var s = new StringBuilder(256); GetWindowText(h, s, 256); return s.ToString();
  }
}
'@
[W]::SetProcessDPIAware() | Out-Null

$results = [System.Collections.Generic.List[string]]::new()
$failed = 0
function Check([string] $name, [bool] $ok, [string] $detail = '') {
  $line = "{0} {1}{2}" -f ($(if ($ok) { 'PASS' } else { 'FAIL' })), $name, $(if ($detail) { " — $detail" } else { '' })
  Write-Host $line
  $results.Add($line)
  if (-not $ok) { $script:failed++ }
}
function Note([string] $text) { Write-Host "NOTE $text"; $results.Add("NOTE $text") }

function Shot([string] $name) {
  $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
  $bmp.Save((Join-Path $Out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
}

$MOVE = 0x0001; $LDOWN = 0x0002; $LUP = 0x0004; $RDOWN = 0x0008; $RUP = 0x0010
function Click([int] $x, [int] $y, [switch] $Right, [switch] $Double) {
  [W]::SetCursorPos($x, $y) | Out-Null; Start-Sleep -Milliseconds 120
  $n = if ($Double) { 2 } else { 1 }
  for ($i = 0; $i -lt $n; $i++) {
    if ($Right) { [W]::mouse_event($RDOWN, 0, 0, 0, [UIntPtr]::Zero); [W]::mouse_event($RUP, 0, 0, 0, [UIntPtr]::Zero) }
    else { [W]::mouse_event($LDOWN, 0, 0, 0, [UIntPtr]::Zero); [W]::mouse_event($LUP, 0, 0, 0, [UIntPtr]::Zero) }
    Start-Sleep -Milliseconds 60
  }
  Start-Sleep -Milliseconds 500
}
function Drag([int] $x, [int] $y, [int] $dx, [int] $dy) {
  [W]::SetCursorPos($x, $y) | Out-Null; Start-Sleep -Milliseconds 200
  [W]::mouse_event($LDOWN, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 150
  for ($i = 1; $i -le 10; $i++) {
    [W]::SetCursorPos($x + [int]($dx * $i / 10), $y + [int]($dy * $i / 10)) | Out-Null
    Start-Sleep -Milliseconds 40
  }
  Start-Sleep -Milliseconds 150
  [W]::mouse_event($LUP, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 600
}
function Keys([byte[]] $vks) {
  foreach ($k in $vks) { [W]::keybd_event($k, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40 }
  [array]::Reverse($vks)
  foreach ($k in $vks) { [W]::keybd_event($k, 0, 2, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40 }
  Start-Sleep -Milliseconds 700
}
function Size($r) { "{0}x{1} at {2},{3}" -f ($r.R - $r.L), ($r.B - $r.T), $r.L, $r.T }

$proc = $null
# The first instance (a second launch with a link forwards it and exits).
function DecayPid { if ($script:proc -and -not $script:proc.HasExited) { $script:proc.Id } }
function WaitWindow([scriptblock] $match, [int] $seconds = 30) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    $decay = DecayPid
    if ($decay) {
      foreach ($h in [W]::Visible([uint32]$decay)) {
        $t = [W]::Title($h); $r = [W]::Bounds($h)
        if (& $match $t $r) { return $h }
      }
    }
    Start-Sleep -Milliseconds 300
  }
  return [IntPtr]::Zero
}
function ListWindows([string] $when) {
  $decay = DecayPid
  if (-not $decay) { Note "${when}: Decay isn't running"; return }
  foreach ($h in [W]::Visible([uint32]$decay)) { Note ("${when}: '{0}' {1}" -f [W]::Title($h), (Size ([W]::Bounds($h)))) }
}

# A bigger desktop than the runner's 1024x768, so half-screen Snap fits Settings (min 820).
Note ("display change: {0}" -f [W]::SetResolution(1920, 1080))
Start-Sleep -Seconds 2
$build = [int](Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuildNumber
$screen = [System.Windows.Forms.Screen]::PrimaryScreen
Note ("Windows build $build; screen {0}x{1}; work area {2}" -f $screen.Bounds.Width, $screen.Bounds.Height, $screen.WorkingArea)
$eleven = $build -ge 22000

# Light theme to start.
$personalize = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
New-Item -Force -Path $personalize | Out-Null
function SetTheme([int] $light) {
  Set-ItemProperty $personalize AppsUseLightTheme $light -Type DWord
  Set-ItemProperty $personalize SystemUsesLightTheme $light -Type DWord
  $r = [IntPtr]::Zero
  [W]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [IntPtr]::Zero, 'ImmersiveColorSet', 2, 3000, [ref]$r) | Out-Null
  Start-Sleep -Seconds 2
}
SetTheme 1
if (-not $eleven) {
  # Windows 10 shell: show every notification icon on the taskbar (no overflow).
  Set-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer' EnableAutoTray 0 -Type DWord
  Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 4
  if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer; Start-Sleep -Seconds 4 }
}

# Debug builds have a console window: out of the way.
function HideConsole {
  $decay = DecayPid
  if (-not $decay) { return }
  foreach ($h in [W]::Visible([uint32]$decay)) {
    if ([W]::Title($h) -like '*decay.exe*') { [W]::ShowWindow($h, 6) | Out-Null }
  }
}
function Backdrop([string] $what, [IntPtr] $h, [int] $expect) {
  if (-not $eleven) { return }
  $type = [W]::DwmInt($h, 38); $corner = [W]::DwmInt($h, 33)
  Check "$what has its backdrop (DWMWA_SYSTEMBACKDROP_TYPE = $expect)" ($type -eq $expect) "read $type"
  Check "$what asks for rounded corners (DWMWA_WINDOW_CORNER_PREFERENCE = 2)" ($corner -eq 2) "read $corner"
}

try {
  # --- 1. A plain first launch: onboarding (fixed size, Close only).
  $proc = Start-Process -FilePath $Exe -PassThru
  $onb = WaitWindow { param($t, $r) $t -like '*Setup*' } 60
  HideConsole
  Check 'onboarding window opens' ($onb -ne [IntPtr]::Zero)
  Start-Sleep -Seconds 3
  Shot '01-onboarding-light'
  if ($onb -ne [IntPtr]::Zero) {
    Backdrop 'onboarding' $onb 2
    $r = [W]::Bounds($onb)
    Note "onboarding: $(Size $r)"
    Drag ($r.L + 300) ($r.T + 16) -80 40
    $m = [W]::Bounds($onb)
    Check 'onboarding moves by its title bar' (($m.L -eq $r.L - 80) -and ($m.T -eq $r.T + 40)) "$(Size $r) -> $(Size $m)"
    Click ($m.R - 23) ($m.T + 16)
    Start-Sleep -Seconds 1
    Check 'onboarding closes with its Close button' (-not [W]::IsWindowVisible($onb))
  }

  # --- 2. Settings (resizable, custom title bar).
  Start-Process -FilePath $Exe -ArgumentList 'decay://settings' | Out-Null
  $set = WaitWindow { param($t, $r) $t -eq 'Settings' } 30
  Check 'settings window opens from a link' ($set -ne [IntPtr]::Zero)
  if ($set -eq [IntPtr]::Zero) { ListWindows 'no settings'; throw 'no settings window' }
  Start-Sleep -Seconds 3
  Shot '02-settings-light'
  Backdrop 'settings' $set 2
  $r = [W]::Bounds($set); Note "settings: $(Size $r)"

  # Move: drag the title bar.
  Drag ($r.L + 300) ($r.T + 16) 30 20
  $m = [W]::Bounds($set)
  Check 'settings moves by its title bar' (($m.L -eq $r.L + 30) -and ($m.T -eq $r.T + 20)) "$(Size $r) -> $(Size $m)"

  # Resize: like any Windows window, the left, right and bottom borders are just outside the
  # visible frame (the shadow's invisible border); the top one is inside it.
  $w0 = $m.R - $m.L; $h0 = $m.B - $m.T
  Drag ($m.R + 3) (($m.T + $m.B) / 2) -60 0
  $a = [W]::Bounds($set)
  Check 'right edge resizes' ([math]::Abs(($a.R - $a.L) - ($w0 - 60)) -le 2) "width $w0 -> $($a.R - $a.L)"
  Drag (($a.L + $a.R) / 2) ($a.B + 3) 0 -40
  $b = [W]::Bounds($set)
  Check 'bottom edge resizes' ([math]::Abs(($b.B - $b.T) - ($h0 - 40)) -le 2) "height $h0 -> $($b.B - $b.T)"
  Drag ($b.R + 3) ($b.B + 3) 30 20
  $c = [W]::Bounds($set)
  Check 'corner resizes both ways' ((($c.R - $c.L) -gt ($b.R - $b.L)) -and (($c.B - $c.T) -gt ($b.B - $b.T))) "$(Size $b) -> $(Size $c)"
  Drag ($c.L - 3) (($c.T + $c.B) / 2) 25 0
  $d = [W]::Bounds($set)
  Check 'left edge resizes' (($d.L -gt $c.L) -and ($d.R -eq $c.R)) "$(Size $c) -> $(Size $d)"
  Drag (($d.L + $d.R) / 2) ($d.T + 1) 0 20
  $e = [W]::Bounds($set)
  Check 'top edge resizes (not a move)' (($e.T -gt $d.T) -and ($e.B -eq $d.B)) "$(Size $d) -> $(Size $e)"

  # Maximise and restore with the caption button (second from the right, 46 px wide).
  Click ($e.R - 46 - 23) ($e.T + 16)
  Start-Sleep -Milliseconds 800
  Check 'maximise button maximises' ([W]::IsZoomed($set))
  Shot '03-settings-maximised'
  $z = [W]::Bounds($set); Note "maximised: $(Size $z)"
  $wa = $screen.WorkingArea
  Check 'maximised fills the work area (not under the taskbar)' (($z.B - $z.T) -le $wa.Height + 2) "work area $($wa.Width)x$($wa.Height)"
  Click ($z.R - 46 - 23) ($z.T + 16)
  Start-Sleep -Milliseconds 800
  Check 'restore button restores' (-not [W]::IsZoomed($set))

  # Double-click the title bar: maximise, then restore.
  $r = [W]::Bounds($set)
  Click ($r.L + 300) ($r.T + 16) -Double
  Start-Sleep -Milliseconds 800
  Check 'double-click on the title bar maximises' ([W]::IsZoomed($set))
  $z = [W]::Bounds($set)
  Click ($z.L + 300) ($z.T + 16) -Double
  Start-Sleep -Milliseconds 800
  Check 'double-click again restores' (-not [W]::IsZoomed($set))

  # Snap: Win+Left fills the left half of the work area.
  $r = [W]::Bounds($set)
  Click ($r.L + 300) ($r.T + 16)
  Keys @(0x5B, 0x25)
  $s = [W]::Bounds($set)
  Shot '04-settings-snapped-left'
  $half = $wa.Width / 2
  Check 'Win+Left snaps to the left half' (([math]::Abs($s.L - $wa.Left) -le 8) -and ([math]::Abs(($s.R - $s.L) - $half) -le 16) -and (($s.B - $s.T) -ge $wa.Height - 16)) (Size $s)
  # Snap by dragging the title bar to the screen's left edge.
  Keys @(0x5B, 0x28)  # Win+Down: back to normal
  $r = [W]::Bounds($set)
  Drag ($r.L + 200) ($r.T + 16) (-($r.L + 200) + 1) 100
  Start-Sleep -Seconds 1
  $s2 = [W]::Bounds($set)
  Shot '05-settings-drag-snapped'
  Check 'dragging to the left edge snaps' (([math]::Abs($s2.L - $wa.Left) -le 8) -and (($s2.B - $s2.T) -ge $wa.Height - 16)) (Size $s2)
  Keys @(0x5B, 0x28)
  # Snap layouts (hovering Maximise) needs the system caption button: a custom one has none.
  Note 'Snap layouts flyout on hovering Maximise: not offered by a page-drawn caption button (known)'

  # Dark theme.
  SetTheme 0
  Start-Sleep -Seconds 2
  Shot '06-settings-dark'

  # --- 3. The search panel.
  Start-Process -FilePath $Exe -ArgumentList 'decay://search?q=test' | Out-Null
  $panel = WaitWindow { param($t, $r) $t -like '*earch*' } 20
  Check 'search panel opens from a link' ($panel -ne [IntPtr]::Zero)
  Start-Sleep -Seconds 2
  Shot '07-panel-dark'
  if ($panel -ne [IntPtr]::Zero) {
    Backdrop 'panel' $panel 3
    $p = [W]::Bounds($panel); Note "panel: $(Size $p)"
    $expect = if ($eleven) { 680 } else { 712 }
    Check "panel is $expect px wide" ([math]::Abs(($p.R - $p.L) - $expect) -le 2) (Size $p)
    Keys @(0x1B)  # Esc
  }
  SetTheme 1
  Start-Process -FilePath $Exe -ArgumentList 'decay://search' | Out-Null
  Start-Sleep -Seconds 3
  Shot '08-panel-light'
  Keys @(0x1B)

  # --- 4. The tray popover.
  Click ($screen.Bounds.Width / 2) 10  # focus away from Decay
  function TrayButton {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = New-Object System.Windows.Automation.PropertyCondition ([System.Windows.Automation.AutomationElement]::ControlTypeProperty), ([System.Windows.Automation.ControlType]::Button)
    foreach ($el in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)) {
      $n = $el.Current.Name
      if ($n -like 'Decay*' -and $n -notlike '*running window*' -and $el.Current.ProcessId -ne (DecayPid)) { return $el }
    }
    return $null
  }
  # Windows 11 puts new icons in the overflow: promote Decay's to the taskbar.
  Get-ChildItem 'HKCU:\Control Panel\NotifyIconSettings' -ErrorAction SilentlyContinue | ForEach-Object {
    $path = (Get-ItemProperty $_.PSPath).ExecutablePath
    if ($path -like '*decay.exe') { Set-ItemProperty $_.PSPath IsPromoted 1 -Type DWord; Note "promoted tray icon ($path)" }
  }
  Start-Sleep -Seconds 2
  $icon = TrayButton
  if (-not $icon) {
    # Windows 10 shell: open the overflow ("Show hidden icons") and look there.
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    $chevron = $all | Where-Object { $_.Current.Name -match 'hidden icons|Notification Chevron' } | Select-Object -First 1
    if ($chevron) {
      $cr = $chevron.Current.BoundingRectangle
      Click ([int]($cr.X + $cr.Width / 2)) ([int]($cr.Y + $cr.Height / 2))
      Start-Sleep -Seconds 1
      $icon = TrayButton
      Note 'tray icon found in the overflow'
    }
  }
  Shot '09-taskbar'
  Check 'tray icon found' ($null -ne $icon) $(if ($icon) { "'$($icon.Current.Name)'" } else { '' })
  if ($icon) {
    $ir = $icon.Current.BoundingRectangle
    $ix = [int]($ir.X + $ir.Width / 2); $iy = [int]($ir.Y + $ir.Height / 2)
    Note ("tray icon at {0},{1} {2}x{3}" -f $ir.X, $ir.Y, $ir.Width, $ir.Height)
    Click $ix $iy
    $pop = WaitWindow { param($t, $r) $t -eq 'Decay' -and ($r.R - $r.L) -lt 500 } 10
    Check 'left click opens the popover' ($pop -ne [IntPtr]::Zero)
    if ($pop -eq [IntPtr]::Zero) { Shot '10-popover-missing'; ListWindows 'after the tray click' }
    Start-Sleep -Seconds 2
    Shot '10-popover-light'
    if ($pop -ne [IntPtr]::Zero) { Backdrop 'popover' $pop 2 }
    if ($pop -ne [IntPtr]::Zero) {
      $q = [W]::Bounds($pop); Note "popover: $(Size $q)"
      $wa = [System.Windows.Forms.Screen]::FromPoint((New-Object System.Drawing.Point $ix, $iy)).WorkingArea
      $inside = ($q.L -ge $wa.Left) -and ($q.T -ge $wa.Top) -and ($q.R -le $wa.Right) -and ($q.B -le $wa.Bottom)
      Check 'popover is inside the work area' $inside "work area $wa"
      $near = ([math]::Abs($q.B - $wa.Bottom) -le 16) -and ($ix -ge $q.L - 8) -and ($ix -le $q.R + 8 -or $q.R -ge $wa.Right - 16)
      Check 'popover sits above the taskbar by the icon' $near
      Check 'popover is 360x500' ((([math]::Abs(($q.R - $q.L) - 360)) -le 2) -and (([math]::Abs(($q.B - $q.T) - 500)) -le 2)) (Size $q)
      # Clicking the icon again closes it (not close-then-reopen).
      Click $ix $iy
      Start-Sleep -Milliseconds 800
      Check 'clicking the icon again closes it' (-not [W]::IsWindowVisible($pop))
      # Opens again on the next click; a click elsewhere closes it.
      Start-Sleep -Milliseconds 500
      Click $ix $iy
      Start-Sleep -Milliseconds 800
      Check 'a third click opens it again' ([W]::IsWindowVisible($pop))
      Click ($wa.Left + 40) ($wa.Top + 40)
      Start-Sleep -Milliseconds 800
      Check 'a click elsewhere closes it' (-not [W]::IsWindowVisible($pop))
      # Dark.
      SetTheme 0
      Click $ix $iy
      Start-Sleep -Seconds 2
      Shot '11-popover-dark'
      Keys @(0x1B)
      Start-Sleep -Milliseconds 600
      Check 'Esc closes it' (-not [W]::IsWindowVisible($pop))
    }
    # Right click: the native menu.
    Click $ix $iy -Right
    Start-Sleep -Seconds 1
    Shot '12-tray-menu'
    Keys @(0x1B)
  }
} catch {
  Check 'script ran to the end' $false "$($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
  Shot '99-error'
} finally {
  ListWindows 'at the end'
  Get-Process decay -ErrorAction SilentlyContinue | Stop-Process -Force
  foreach ($dir in @((Join-Path $env:APPDATA 'Decay'), (Join-Path $env:APPDATA 'com.decay.app'), (Join-Path $env:LOCALAPPDATA 'Decay'))) {
    if (Test-Path $dir) {
      Get-ChildItem $dir -Recurse -Include *.log, *.txt -ErrorAction SilentlyContinue | Copy-Item -Destination $Out -ErrorAction SilentlyContinue
    }
  }
  $results | Set-Content (Join-Path $Out 'results.txt')
}
Write-Host "$failed check(s) failed"
exit $(if ($failed) { 1 } else { 0 })
