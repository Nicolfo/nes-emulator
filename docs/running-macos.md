# Running the emulator on macOS

This guide covers running the prebuilt `nes-emulator-macos-universal` release
archive. The binary is universal: it runs natively on both Apple Silicon and
Intel Macs. To build from source instead, see the main `README.md` (included in this archive).

## Steps

1. Extract `nes-emulator-macos-universal.zip` (double-click in Finder, or
   `unzip` in Terminal) and open a Terminal in the extracted directory.
2. Remove the Gatekeeper quarantine flag - the binary is not notarized, so
   macOS will otherwise refuse to run a file downloaded from the internet:

   ```
   xattr -d com.apple.quarantine nes-emulator
   ```

3. Run it:

   ```
   ./nes-emulator                  # opens the home menu
   ./nes-emulator path/to/rom.nes  # boots the ROM directly
   ```

   If the binary lost its executable bit during extraction, restore it with
   `chmod +x nes-emulator`.

## Notes

- If macOS still blocks the launch, allow it under *System Settings → Privacy
  & Security → Open Anyway*, or right-click the binary in Finder and choose
  *Open*.
- The dock shows a generic icon: a custom dock icon requires an `.app` bundle,
  which this bare binary distribution does not include.
- **Settings**: key bindings and window scale are saved to
  `nes-emulator-config.json`, created in the directory the emulator is
  launched from.
- **Controls**: see the *Default controls* table in the included `README.md`.
  All bindings except Escape can be changed in Settings.
