# Running the emulator on macOS

This guide covers running the prebuilt `nes-emulator-macos-universal` release
archive. It ships as a `NES Emulator.app` bundle that is universal: it runs
natively on both Apple Silicon and Intel Macs. To build from source instead,
see the main `README.md` (included in this archive).

## Steps

1. Extract `nes-emulator-macos-universal.zip` (double-click in Finder, or
   `unzip` in Terminal).
2. The app is not notarized, so Gatekeeper quarantines anything downloaded
   from the internet. Clear the quarantine flag so it will launch:

   ```
   xattr -dr com.apple.quarantine "NES Emulator.app"
   ```

3. Double-click **NES Emulator.app** in Finder. The home menu opens, where you
   can load a ROM with a native file picker, change settings, or quit.

   To boot a ROM directly from a terminal, invoke the binary inside the bundle:

   ```
   "NES Emulator.app/Contents/MacOS/nes-emulator" path/to/rom.nes
   ```

## Notes

- If macOS still blocks the launch, right-click the app in Finder and choose
  *Open*, or allow it under *System Settings → Privacy & Security → Open
  Anyway*.
- **Dock icon**: the bundle includes the emulator icon, so it shows in the dock
  and Finder instead of a generic one.
- **Settings**: key bindings and window scale are saved to
  `~/Library/Application Support/nes-emulator/config.json`, so they persist
  whether you launch the app from Finder or run the inner binary from a
  terminal.
- **Controls**: see the *Default controls* table in the included `README.md`.
  All bindings except Escape can be changed in Settings.
