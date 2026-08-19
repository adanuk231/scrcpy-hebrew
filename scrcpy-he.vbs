' ---------------------------------------------------------------------------
'  scrcpy with working Hebrew input, started with no console window at all -
'  not even the brief flash the .cmd gives you. Point desktop shortcuts here:
'
'      wscript.exe "...\scrcpy-he.vbs" -s SERIAL --stay-awake
'
'  scrcpy's own output goes to %LOCALAPPDATA%\scrcpy-hebrew\scrcpy-<serial>.log
' ---------------------------------------------------------------------------
Option Explicit
Dim fso, sh, here, exe, cmd, i
Set fso = CreateObject("Scripting.FileSystemObject")
Set sh  = CreateObject("WScript.Shell")

here = fso.GetParentFolderName(WScript.ScriptFullName)

' pyw.exe is the windowless twin of the py launcher; pythonw is the fallback
exe = sh.ExpandEnvironmentStrings("%WINDIR%\pyw.exe")
If Not fso.FileExists(exe) Then exe = "pythonw.exe"

cmd = """" & exe & """ """ & fso.BuildPath(here, "scrcpy_hebrew.py") & """ launch"
For i = 0 To WScript.Arguments.Count - 1
    cmd = cmd & " """ & WScript.Arguments(i) & """"
Next

sh.Run cmd, 0, False        ' 0 = hidden, False = do not wait
