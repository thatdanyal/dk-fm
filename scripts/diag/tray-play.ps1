# Diagnostic: play two short generated songs, close to the tray while playing (song changes
# happen while hidden), reopen it (single instance), close again; CPU at each step.
param([string]$Exe = "$PSScriptRoot\..\..\target\release\dkfm.exe")
Add-Type @"
using System; using System.Runtime.InteropServices;
public class DkWin2 { public delegate bool EP(IntPtr h, IntPtr l);
[DllImport("user32.dll")] public static extern bool EnumWindows(EP f, IntPtr l);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
[DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
[DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
public static IntPtr Main(uint pid){ IntPtr r=IntPtr.Zero; EnumWindows((h,l)=>{uint p; GetWindowThreadProcessId(h,out p); var c=new System.Text.StringBuilder(256); GetClassName(h,c,256); if(p==pid && c.ToString()=="Window Class"){r=h;} return true;}, IntPtr.Zero); return r; } }
"@
function Cpu($id, $secs) { $q = Get-Process -Id $id -ErrorAction SilentlyContinue; if (-not $q) { return "exited" }; $a = $q.CPU; Start-Sleep $secs; $q.Refresh(); "{0:N0}% of a core, private {1} MB" -f (($q.CPU - $a) / $secs * 100), [int]($q.PrivateMemorySize64 / 1MB) }
$root = Join-Path $env:TEMP ("dkfm-diag-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
$music = Join-Path $root "music"; $data = Join-Path $root "data"
New-Item -ItemType Directory -Force $music, $data | Out-Null
python -c "import wave,struct,math,sys
for i,f in enumerate([330,440,550]):
  w=wave.open(sys.argv[1]+'/tone%d.wav'%i,'wb'); w.setnchannels(2); w.setsampwidth(2); w.setframerate(44100)
  w.writeframes(b''.join(struct.pack('<hh',int(3000*math.sin(2*math.pi*f*n/44100)),int(3000*math.sin(2*math.pi*f*n/44100))) for n in range(44100*6))); w.close()" $music
$m = $music.Replace([string][char]92, '/')
Set-Content (Join-Path $data "settings.json") ('{"closeMode":"playing","onboarded":true,"autoUpdate":false,"musicFolders":["' + $m + '"],"musicFoldersSet":true,"downloadDir":"' + $m + '"}')
$env:DKFM_USER_DATA = $data
# first run: scan the test songs into the library, then quit (X with nothing playing quits)
$p = Start-Process $Exe -PassThru
Start-Sleep 8
[DkWin2]::PostMessage([DkWin2]::Main($p.Id), 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Start-Sleep 3
$env:DKFM_TEST_PLAY = "1"
$p = Start-Process $Exe -PassThru
Start-Sleep 6
"visible, playing: " + (Cpu $p.Id 3)
$h = [DkWin2]::Main($p.Id)
[DkWin2]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Start-Sleep 2
"tray, playing:    " + (Cpu $p.Id 6) + "  (songs change while hidden)"
"tray, playing:    " + (Cpu $p.Id 6) + "  visible=" + [DkWin2]::IsWindowVisible($h)
"tray, finished:   " + (Cpu $p.Id 6)
Remove-Item Env:\DKFM_TEST_PLAY
Start-Process $Exe | Out-Null   # second launch = show the running copy
Start-Sleep 3
"reopened:         " + (Cpu $p.Id 3) + "  visible=" + [DkWin2]::IsWindowVisible($h)
[DkWin2]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Start-Sleep 3
"X, nothing playing: " + (Cpu $p.Id 2)
"root=$root"
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
