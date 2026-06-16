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

## Adding it to your application menu (optional)

The archive bundles a launcher entry (`nes-emulator.desktop`), an icon
(`nes-emulator.png`) and an installer. Running it installs everything into
`~/.local` (no root required), so "NES Emulator" shows up in your desktop's
app launcher with its icon:

```
./install.sh
```

To install system-wide instead, set a prefix: `PREFIX=/usr/local sudo ./install.sh`.
The icon shows on both X11 and Wayland because the emulator sets its window
`app_id`/`WM_CLASS` to `nes-emulator`, matching the launcher entry.

## Notes

- **Settings**: key bindings and window scale are saved to
  `nes-emulator-config.json`, created in the directory the emulator is
  launched from.
- **Controls**: see the *Default controls* table in the included `README.md`.
  All bindings except Escape can be changed in Settings.
