# Running the emulator on Windows

This guide covers running the prebuilt `nes-emulator-windows-x64` release
archive. To build from source instead, see the main `README.md` (included in this archive).

## Steps

1. Extract `nes-emulator-windows-x64.zip` anywhere (right-click → *Extract
   All...*).
2. Double-click `nes-emulator.exe`. The home menu opens, where you can load a
   ROM with a native file picker, change settings, or quit.
3. Alternatively, boot a ROM directly from a terminal:

   ```
   .\nes-emulator.exe path\to\rom.nes
   ```

## Notes

- **SmartScreen warning**: the binary is not code-signed, so the first launch
  may show "Windows protected your PC". Click *More info* → *Run anyway*.
- **No console window**: release builds use the Windows GUI subsystem, so only
  the emulator window opens - no separate command-prompt window appears behind
  it. (Trace/log output via the `NES_*_LOG` env vars is available in debug
  builds only.)
- **Settings**: key bindings and window scale are saved to
  `nes-emulator-config.json`, created in the directory the emulator is
  launched from.
- **Controls**: see the *Default controls* table in the included `README.md`.
  All bindings except Escape can be changed in Settings.
