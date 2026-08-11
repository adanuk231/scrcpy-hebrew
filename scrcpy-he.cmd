@echo off
rem ---------------------------------------------------------------------------
rem  scrcpy with working Hebrew (and other non-Latin) input.
rem  Probes the phone, starts scrcpy in the right keyboard mode, and brings up
rem  the input daemon. Pass any scrcpy arguments through as usual:
rem
rem      scrcpy-he.cmd -s SERIAL --stay-awake
rem ---------------------------------------------------------------------------
where /q py.exe && (set "PY=py -3") || (set "PY=python")
%PY% "%~dp0scrcpy_hebrew.py" launch %*
