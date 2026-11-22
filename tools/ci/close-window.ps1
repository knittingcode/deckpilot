# Finds or closes a process's visible top-level window titled DeckPilot.
#
#   close-window.ps1 -AppPid <pid> -Mode find    exit 0 and print the window once shown
#   close-window.ps1 -AppPid <pid> -Mode close   WM_SYSCOMMAND SC_CLOSE, what the
#                                                title bar's close button sends
#
# The window turns SC_CLOSE into WM_CLOSE, which Tauri reports as the
# window's CloseRequested.
param(
  [Parameter(Mandatory = $true)][int]$AppPid,
  [Parameter(Mandatory = $true)][ValidateSet('find', 'close')][string]$Mode
)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class CloseWindowNative {
  delegate bool EnumProc(IntPtr hwnd, IntPtr lParam);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr lParam);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int max);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder text, int max);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);

  public static List<string> Find(uint pid, string title) {
    var found = new List<string>();
    EnumWindows((hwnd, _) => {
      uint owner;
      GetWindowThreadProcessId(hwnd, out owner);
      if (owner != pid || !IsWindowVisible(hwnd)) return true;
      var text = new StringBuilder(256);
      GetWindowText(hwnd, text, text.Capacity);
      if (text.ToString() != title) return true;
      var cls = new StringBuilder(256);
      GetClassName(hwnd, cls, cls.Capacity);
      found.Add(hwnd.ToInt64() + "\t" + cls);
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
'@

$windows = [CloseWindowNative]::Find([uint32]$AppPid, 'DeckPilot')
if ($windows.Count -eq 0) { exit 1 }
$hwnd, $class = $windows[0] -split "`t"
if ($Mode -eq 'close') {
  $WM_SYSCOMMAND = 0x0112
  $SC_CLOSE = 0xF060
  if (-not [CloseWindowNative]::PostMessage([IntPtr][long]$hwnd, $WM_SYSCOMMAND, [IntPtr]$SC_CLOSE, [IntPtr]::Zero)) {
    Write-Error "PostMessage to hwnd $hwnd failed"
  }
  "posted WM_SYSCOMMAND SC_CLOSE to hwnd $hwnd (class '$class', title 'DeckPilot')"
} else {
  "hwnd $hwnd (class '$class', title 'DeckPilot') of pid $AppPid"
}
