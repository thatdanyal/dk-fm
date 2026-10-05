# Diagnostic: start a local build on an isolated data folder, close it to the tray, measure CPU.
param([string]$Exe = "$PSScriptRoot\..\..\target\release\dkfm.exe", [string]$Mode = "tray")
Add-Type @"
using System; using System.Runtime.InteropServices;
public class DkWin { public delegate bool EP(IntPtr h, IntPtr l);
[DllImport("user32.dll")] public static extern bool EnumWindows(EP f, IntPtr l);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
[DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
[DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
public static IntPtr Main(uint pid){ IntPtr r=IntPtr.Zero; EnumWindows((h,l)=>{uint p; GetWindowThreadProcessId(h,out p); var c=new System.Text.StringBuilder(256); GetClassName(h,c,256); if(p==pid && c.ToString()=="Window Class"){r=h;} return true;}, IntPtr.Zero); return r; } }
"@
function Cpu($id, $secs) { $q = Get-Process -Id $id -ErrorAction SilentlyContinue; if (-not $q) { return "exited" }; $a = $q.CPU; Start-Sleep $secs; $q.Refresh(); "{0:N0}% of a core, private {1} MB" -f (($q.CPU - $a) / $secs * 100), [int]($q.PrivateMemorySize64 / 1MB) }
$data = Join-Path $env:TEMP ("dkfm-diag-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
New-Item -ItemType Directory -Force $data | Out-Null
Set-Content (Join-Path $data "settings.json") ('{"closeMode":"' + $Mode + '","onboarded":true,"autoUpdate":false,"musicFolders":[],"musicFoldersSet":true}')
$env:DKFM_USER_DATA = $data
$p = Start-Process $Exe -PassThru
Start-Sleep 5
"visible, idle:   " + (Cpu $p.Id 3)
[DkWin]::PostMessage([DkWin]::Main($p.Id), 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Start-Sleep 2
"after X:         " + (Cpu $p.Id 5)
"after X, +5s:    " + (Cpu $p.Id 5)
"pid=$($p.Id) data=$data"
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
