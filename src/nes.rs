use crate::bus::Bus;
use crate::cartridge::{Region, load_rom};
use crate::cpu::Cpu;
use crate::mapper::NsfMapper;
use crate::nsf::{self, NsfInfo};
use crate::savestate::{MAGIC, SaveState, VERSION};

pub struct Nes {
    pub cpu: Cpu,
    region: Region,
    battery: bool,
}

impl Nes {
    /// Build a machine from an iNES/NES 2.0 ROM image, or from an NSF music
    /// file (detected by its magic; see [`Nes::new_nsf`]).
    pub fn new(rom: &[u8]) -> Result<Self, String> {
        if nsf::is_nsf(rom) {
            return Self::new_nsf(rom);
        }
        let (cart, region, battery) = load_rom(rom)?;
        let mut cpu = Cpu::new(Bus::with_region(cart, region));
        cpu.reset();
        Ok(Nes {
            cpu,
            region,
            battery,
        })
    }

    /// Boot a Famicom Disk System image. `disk` is a fwNES (`FDS`) or
    /// headerless `.fds` image; `bios` is the 8KB `disksys.rom`. Side A of
    /// the first disk starts inserted. The disk counts as battery RAM: its
    /// `.fds`-layout bytes (sides x 65500, no header) round-trip through
    /// [`Nes::battery_ram`] / [`Nes::load_battery_ram`], so disk saves persist
    /// through the usual `.sav` file.
    pub fn new_fds(disk: &[u8], bios: &[u8]) -> Result<Self, String> {
        if bios.len() != 0x2000 {
            return Err(format!("FDS BIOS must be 8192 bytes, got {}", bios.len()));
        }
        let sides = crate::mapper::parse_fds_image(disk)?;
        let cart = Box::new(crate::mapper::Fds::new(bios.to_vec(), sides));
        let region = Region::Ntsc;
        let mut cpu = Cpu::new(Bus::with_region(cart, region));
        cpu.reset();
        Ok(Nes {
            cpu,
            region,
            battery: true,
        })
    }

    /// Number of disk sides when an FDS image is loaded; None for cartridges.
    pub fn fds_side_count(&self) -> Option<usize> {
        self.cpu.bus.cart.fds_side_count()
    }

    /// Disk side currently in the drive; None when ejected, mid-swap, or not
    /// running an FDS image.
    pub fn fds_current_side(&self) -> Option<usize> {
        self.cpu.bus.cart.fds_current_side()
    }

    /// Eject the disk (None) or insert a side (0 = disk 1 side A, 1 = side B,
    /// ...). The new side appears after a ~1 s empty-drive delay so the BIOS
    /// sees the disk change. No-op for cartridges.
    pub fn fds_insert(&mut self, side: Option<usize>) {
        self.cpu.bus.cart.fds_insert(side);
    }

    /// Build a machine that plays an NSF music file. The NSF board's driver
    /// starts the header's default song at power-on; switch tracks with
    /// [`Nes::nsf_play`].
    pub fn new_nsf(data: &[u8]) -> Result<Self, String> {
        let (header, body) = nsf::parse(data)?;
        let region = header.region;
        let cart = Box::new(NsfMapper::new(&header, body));
        let mut cpu = Cpu::new(Bus::with_region(cart, region));
        cpu.reset();
        Ok(Nes {
            cpu,
            region,
            battery: false,
        })
    }

    /// NSF metadata (title, song count, current song) when an NSF is loaded.
    pub fn nsf_info(&self) -> Option<NsfInfo> {
        self.cpu.bus.cart.nsf_info()
    }

    /// Restart an NSF machine into song `song` (0-based, clamped to the
    /// song count): the board rewinds its banking and the CPU/PPU/APU are
    /// reset, so the driver clears RAM and runs the tune's init again.
    /// No-op when the loaded image isn't an NSF.
    pub fn nsf_play(&mut self, song: usize) {
        if self.nsf_info().is_none() {
            return;
        }
        self.cpu.bus.cart.nsf_select(song);
        self.cpu.bus.ppu.reset();
        self.cpu.bus.apu.reset();
        self.cpu.reset();
    }

    pub fn region(&self) -> Region {
        self.region
    }

    /// Soft reset (the console's RESET button): pulses the PPU and APU reset
    /// lines and re-runs the CPU reset sequence, so execution resumes from the
    /// cartridge's reset vector. Cartridge banking, PRG/CHR RAM, VRAM, palette
    /// and OAM are all preserved - exactly what the RESET line does on
    /// hardware; the game's reset handler reinitializes everything else.
    pub fn reset(&mut self) {
        self.cpu.bus.ppu.reset();
        self.cpu.bus.apu.reset();
        self.cpu.reset();
    }

    /// Battery-backed PRG RAM to persist as a .sav file; None when the
    /// cartridge has no battery or the board has no PRG RAM.
    pub fn battery_ram(&self) -> Option<&[u8]> {
        if self.battery {
            self.cpu.bus.cart.prg_ram()
        } else {
            None
        }
    }

    /// Restore a previously saved .sav file into battery-backed PRG RAM.
    /// Ignored when the cartridge has no battery; size mismatches copy the
    /// overlapping prefix.
    pub fn load_battery_ram(&mut self, data: &[u8]) {
        if !self.battery {
            return;
        }
        if let Some(ram) = self.cpu.bus.cart.prg_ram_mut() {
            let n = ram.len().min(data.len());
            ram[..n].copy_from_slice(&data[..n]);
        }
        self.cpu.bus.cart.battery_ram_loaded();
    }

    /// Serialize the full machine state to a savestate blob. The state is
    /// only meaningful for the currently loaded ROM.
    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let state = SaveState {
            magic: MAGIC,
            version: VERSION,
            region: self.region,
            cpu: self.cpu.save_state(),
            bus: self.cpu.bus.save_state(),
            ppu: self.cpu.bus.ppu.clone(),
            apu: self.cpu.bus.apu.save_state(),
            controller: self.cpu.bus.controller1.clone(),
            mapper: self.cpu.bus.cart.save_state(),
        };
        serde_json::to_vec(&state).map_err(|e| format!("failed to serialize savestate: {e}"))
    }

    /// Restore a savestate blob produced by [`Nes::save_state`]. Rejects blobs
    /// with the wrong magic/version or a TV region that doesn't match the
    /// loaded ROM; mapper state from a different ROM is rejected by the mapper.
    /// Host audio configuration (sample rate, filters) is preserved.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: SaveState =
            serde_json::from_slice(data).map_err(|e| format!("not a valid savestate: {e}"))?;
        if state.magic != MAGIC {
            return Err("not a savestate (bad magic)".into());
        }
        if state.version != VERSION {
            return Err(format!(
                "unsupported savestate version {} (expected {VERSION})",
                state.version
            ));
        }
        if state.region != self.region {
            return Err("savestate is for a different TV region".into());
        }
        self.cpu.bus.cart.load_state(&state.mapper)?;
        self.cpu.load_state(state.cpu);
        self.cpu.bus.load_state(state.bus);
        self.cpu.bus.ppu = state.ppu;
        self.cpu.bus.apu.load_state(state.apu);
        self.cpu.bus.controller1 = state.controller;
        Ok(())
    }

    /// Run until the PPU enters vblank (one full frame).
    pub fn run_frame(&mut self) {
        self.cpu.bus.ppu.frame_complete = false;
        while !self.cpu.bus.ppu.frame_complete {
            self.cpu.step();
        }
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.cpu.bus.ppu.framebuffer
    }

    /// Set the host audio output rate (resets the APU filter chain).
    pub fn set_sample_rate(&mut self, hz: f64) {
        self.cpu.bus.apu.set_sample_rate(hz);
    }

    /// Nudge the resampling ratio for dynamic rate control.
    pub fn tune_audio(&mut self, hz: f64) {
        self.cpu.bus.apu.tune(hz);
    }

    /// Drain audio samples generated since the last call.
    pub fn take_audio(&mut self) -> Vec<f32> {
        self.cpu.bus.apu.take_samples()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal 32KB iNES image; flags6 sets the mapper low nibble plus
    /// mirroring/battery/trainer bits.
    fn rom(flags6: u8) -> Vec<u8> {
        let mut data = vec![0u8; 16 + 32 * 1024 + 8 * 1024];
        data[0..4].copy_from_slice(b"NES\x1A");
        data[4] = 2; // 2x 16KB PRG
        data[5] = 1; // 8KB CHR
        data[6] = flags6;
        data
    }

    #[test]
    fn battery_ram_roundtrip() {
        // mapper 1 (MMC1) with the battery bit set
        let mut nes = Nes::new(&rom(0x12)).unwrap();
        nes.load_battery_ram(&[0xAB; 0x2000]);
        let ram = nes.battery_ram().unwrap();
        assert_eq!(ram.len(), 0x2000);
        assert!(ram.iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn savestate_roundtrip_is_deterministic() {
        // Saving, diverging, then restoring must reproduce the exact same
        // machine: run the restored state forward and compare framebuffers.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nestest.nes");
        let data = std::fs::read(path).unwrap();

        let mut nes = Nes::new(&data).unwrap();
        for _ in 0..20 {
            nes.run_frame();
        }
        let blob = nes.save_state().unwrap();

        // Reference: continue from the snapshot point.
        for _ in 0..30 {
            nes.run_frame();
        }
        let reference = nes.framebuffer().to_vec();

        // Restore the snapshot into a fresh machine and run the same steps.
        let mut restored = Nes::new(&data).unwrap();
        restored.load_state(&blob).unwrap();
        for _ in 0..30 {
            restored.run_frame();
        }
        assert_eq!(restored.framebuffer(), reference.as_slice());
    }

    #[test]
    fn soft_reset_reenters_reset_vector_and_clears_ppu_regs() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nestest.nes");
        let data = std::fs::read(path).unwrap();
        let mut nes = Nes::new(&data).unwrap();
        // Run a while so the CPU is deep in the program and PPU regs are set.
        for _ in 0..10 {
            nes.run_frame();
        }
        let reset_vec = {
            let lo = nes.cpu.bus.cart.cpu_read(0xFFFC) as u16;
            let hi = nes.cpu.bus.cart.cpu_read(0xFFFD) as u16;
            (hi << 8) | lo
        };
        nes.reset();
        assert_eq!(nes.cpu.pc, reset_vec, "reset re-enters the reset vector");
        assert_eq!(nes.cpu.bus.ppu.ctrl, 0, "PPUCTRL cleared on reset");
        assert_eq!(nes.cpu.bus.ppu.mask, 0, "PPUMASK cleared on reset");
    }

    #[test]
    fn load_state_rejects_garbage() {
        let mut nes = Nes::new(&rom(0x10)).unwrap();
        assert!(nes.load_state(b"not a savestate").is_err());
    }

    #[test]
    fn no_battery_means_no_sav() {
        // same board, battery bit clear
        let mut nes = Nes::new(&rom(0x10)).unwrap();
        nes.load_battery_ram(&[0xAB; 0x2000]); // ignored
        assert!(nes.battery_ram().is_none());
        assert_eq!(nes.cpu.bus.cart.prg_ram().unwrap()[0], 0);
    }

    /// Synthetic BIOS: an endless `JMP $E000` with every vector at $E000.
    fn tiny_bios() -> Vec<u8> {
        let mut b = vec![0xEA; 0x2000];
        b[0..3].copy_from_slice(&[0x4C, 0x00, 0xE0]);
        for v in [0x1FFA, 0x1FFC, 0x1FFE] {
            b[v] = 0x00;
            b[v + 1] = 0xE0;
        }
        b
    }

    fn tiny_disk() -> Vec<u8> {
        let mut d = b"FDS".to_vec();
        d.resize(16, 0);
        d.push(0x01);
        d.extend_from_slice(b"*NINTENDO-HVC*");
        d.resize(16 + crate::mapper::FDS_SIDE_SIZE, 0);
        d
    }

    #[test]
    fn fds_rejects_bad_bios_size() {
        assert!(Nes::new_fds(&tiny_disk(), &[0; 0x1000]).is_err());
        assert!(Nes::new_fds(b"not a disk", &tiny_bios()).is_err());
    }

    #[test]
    fn fds_boots_and_runs_frames() {
        let mut nes = Nes::new_fds(&tiny_disk(), &tiny_bios()).unwrap();
        assert_eq!(nes.region(), Region::Ntsc);
        assert_eq!(nes.fds_side_count(), Some(1));
        assert_eq!(nes.fds_current_side(), Some(0));
        for _ in 0..10 {
            nes.run_frame();
        }
        let disk = nes.battery_ram().unwrap().to_vec();
        assert_eq!(disk.len(), crate::mapper::FDS_SIDE_SIZE);
        assert_eq!(&disk[1..15], b"*NINTENDO-HVC*");

        // Savestates carry the disk; the restore lands on the same frame.
        let blob = nes.save_state().unwrap();
        nes.run_frame();
        let reference = nes.framebuffer().to_vec();
        let mut restored = Nes::new_fds(&tiny_disk(), &tiny_bios()).unwrap();
        restored.load_state(&blob).unwrap();
        restored.run_frame();
        assert_eq!(restored.framebuffer(), reference.as_slice());

        nes.fds_insert(None);
        assert_eq!(nes.fds_current_side(), None);
    }

    #[test]
    fn cartridges_report_no_fds() {
        let nes = Nes::new(&rom(0x10)).unwrap();
        assert_eq!(nes.fds_side_count(), None);
        assert_eq!(nes.fds_current_side(), None);
    }

    // --- NSF -----------------------------------------------------------------

    /// NSF header: 3 songs (start at song 2), load/init at $8000, play at
    /// $8020. `banks` are the $70-$77 bankswitch-init bytes.
    fn nsf_header(banks: [u8; 8], speed_us: u16) -> Vec<u8> {
        let mut h = vec![0u8; crate::nsf::HEADER_LEN];
        h[0..5].copy_from_slice(b"NESM");
        h[5] = 1;
        h[6] = 3;
        h[7] = 2;
        h[0x08..0x0A].copy_from_slice(&0x8000u16.to_le_bytes());
        h[0x0A..0x0C].copy_from_slice(&0x8000u16.to_le_bytes());
        h[0x0C..0x0E].copy_from_slice(&0x8020u16.to_le_bytes());
        h[0x0E..0x14].copy_from_slice(b"TONE 1");
        h[0x6E..0x70].copy_from_slice(&speed_us.to_le_bytes());
        h[0x70..0x78].copy_from_slice(&banks);
        h
    }

    /// Unbanked NSF whose init stores A/X to $11/$12 and starts a constant
    /// pulse-1 tone; play increments $10.
    fn tone_nsf(speed_us: u16) -> Vec<u8> {
        let mut data = nsf_header([0; 8], speed_us);
        let mut prg = vec![0u8; 0x30];
        let init = [
            0x85, 0x11, // STA $11 (song)
            0x86, 0x12, // STX $12 (region)
            0xA9, 0xBF, 0x8D, 0x00, 0x40, // duty 2, halt, constant volume 15
            0xA9, 0x40, 0x8D, 0x02, 0x40, // period lo
            0xA9, 0x00, 0x8D, 0x03, 0x40, // period hi + length load
            0x60, // RTS
        ];
        prg[..init.len()].copy_from_slice(&init);
        prg[0x20..0x23].copy_from_slice(&[0xE6, 0x10, 0x60]); // INC $10 / RTS
        data.extend_from_slice(&prg);
        data
    }

    #[test]
    fn nsf_plays_at_header_rate_with_sound() {
        let mut nes = Nes::new(&tone_nsf(0)).unwrap(); // auto-detected NSF
        nes.set_sample_rate(48_000.0);
        let info = nes.nsf_info().unwrap();
        assert_eq!(info.title, "TONE 1");
        assert_eq!(info.total_songs, 3);
        assert_eq!(info.current_song, 1);
        // The driver's RAM clear takes a few frames before init runs, so
        // measure the play rate over a steady-state window.
        for _ in 0..10 {
            nes.run_frame();
        }
        let start = nes.cpu.bus.ram[0x10];
        for _ in 0..60 {
            nes.run_frame();
        }
        let ram = &nes.cpu.bus.ram;
        assert_eq!(ram[0x11], 1, "init got the 0-based song in A");
        assert_eq!(ram[0x12], 0, "init got NTSC in X");
        // Default 16639 us period ~= one call per frame.
        let calls = ram[0x10].wrapping_sub(start);
        assert!((59..=61).contains(&calls), "play calls: {calls}");
        let audio = nes.take_audio();
        let peak = audio.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.05, "tone should be audible, peak {peak}");

        // A custom 8320 us speed doubles the play rate.
        let mut fast = Nes::new_nsf(&tone_nsf(8320)).unwrap();
        for _ in 0..10 {
            fast.run_frame();
        }
        let start = fast.cpu.bus.ram[0x10];
        for _ in 0..60 {
            fast.run_frame();
        }
        let n = fast.cpu.bus.ram[0x10].wrapping_sub(start);
        assert!((119..=121).contains(&n), "fast play calls: {n}");
    }

    #[test]
    fn nsf_play_switches_song_and_reinits() {
        let mut nes = Nes::new_nsf(&tone_nsf(0)).unwrap();
        for _ in 0..30 {
            nes.run_frame();
        }
        nes.nsf_play(2);
        assert_eq!(nes.nsf_info().unwrap().current_song, 2);
        for _ in 0..6 {
            nes.run_frame();
        }
        let ram = &nes.cpu.bus.ram;
        assert_eq!(ram[0x11], 2, "init re-run with the new song");
        assert!(ram[0x10] <= 6, "driver cleared RAM, counter {}", ram[0x10]);
        nes.nsf_play(99);
        assert_eq!(nes.nsf_info().unwrap().current_song, 2, "clamped");
    }

    #[test]
    fn nsf_bankswitching_maps_requested_banks() {
        // Three 4KB banks, each filled with $B0 + bank number; bank 0 holds
        // the code. Init maps bank 2 at $9000 and copies its marker to $13.
        let mut data = nsf_header([0, 1, 0, 0, 0, 0, 0, 2], 0);
        let mut prg: Vec<u8> = (0..3 * 0x1000).map(|i| 0xB0 + (i / 0x1000) as u8).collect();
        let init = [
            0x85, 0x11, // STA $11
            0xA9, 0x02, 0x8D, 0xF9, 0x5F, // LDA #2 / STA $5FF9
            0xAD, 0x00, 0x90, 0x85, 0x13, // LDA $9000 / STA $13
            0x60,
        ];
        prg[..init.len()].copy_from_slice(&init);
        prg[0x20..0x23].copy_from_slice(&[0xE6, 0x10, 0x60]);
        data.extend_from_slice(&prg);
        let mut nes = Nes::new_nsf(&data).unwrap();
        // Power-on banking from the header ($F000 = bank 2, vectors aside).
        assert_eq!(nes.cpu.bus.cart.cpu_read(0x9000), 0xB1);
        assert_eq!(nes.cpu.bus.cart.cpu_read(0xF000), 0xB2);
        for _ in 0..6 {
            nes.run_frame();
        }
        assert_eq!(nes.cpu.bus.ram[0x13], 0xB2, "init saw bank 2 at $9000");
        assert!(nes.cpu.bus.ram[0x10] >= 1, "play ran from bank 0");
        // Re-selecting a song restores the header banking.
        nes.nsf_play(0);
        assert_eq!(nes.cpu.bus.cart.cpu_read(0x9000), 0xB1);
    }

    #[test]
    fn nsf_savestate_roundtrip() {
        let data = tone_nsf(0);
        let mut nes = Nes::new_nsf(&data).unwrap();
        for _ in 0..10 {
            nes.run_frame();
        }
        let blob = nes.save_state().unwrap();
        for _ in 0..20 {
            nes.run_frame();
        }
        let reference = nes.cpu.bus.ram;

        let mut restored = Nes::new_nsf(&data).unwrap();
        restored.load_state(&blob).unwrap();
        for _ in 0..20 {
            restored.run_frame();
        }
        assert_eq!(restored.cpu.bus.ram, reference);
        assert!(reference[0x10] >= 20, "play kept running");
    }

    #[test]
    fn nsf_play_is_noop_for_cartridges() {
        let mut nes = Nes::new(&rom(0x10)).unwrap();
        assert!(nes.nsf_info().is_none());
        let pc = nes.cpu.pc;
        nes.nsf_play(1);
        assert_eq!(nes.cpu.pc, pc);
    }
}
