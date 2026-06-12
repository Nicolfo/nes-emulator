# Running the emulator on Linux

This guide covers running the prebuilt `nes-emulator-linux-x64` release
archive. To build from source instead, see the main `README.md` (included in this archive).

## Steps

1. Extract the archive and enter the directory:

   ```
   tar -xzf nes-emulator-linux-x64.tar.gz
   cd nes-emulator-linux-x64
   ```

2. Install the runtime libraries if they are not already present. On a typical
   desktop install only ALSA and GTK 3 may be missing:

   ```
   sudo apt install libasound2 libgtk-3-0      # Debian / Ubuntu (use the
                                               # *t64 package names on 24.04+)
   sudo dnf install alsa-lib gtk3              # Fedora
   ```

   ALSA provides audio output; GTK 3 backs the native *Load ROM* file picker.

3. Run it:

   ```
   ./nes-emulator                  # opens the home menu
   ./nes-emulator path/to/rom.nes  # boots the ROM directly
   ```

   If the binary lost its executable bit during extraction, restore it with
   `chmod +x nes-emulator`.

## Notes

- **Settings**: key bindings and window scale are saved to
  `nes-emulator-config.json`, created in the directory the emulator is
  launched from.
- **Controls**: see the *Default controls* table in the included `README.md`.
  All bindings except Escape can be changed in Settings.
