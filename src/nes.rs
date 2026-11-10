use crate::bus::Bus;
use crate::cartridge::{Region, load_rom};
use crate::cheats::{Cheat, RomPatch};
use crate::controller::InputDevice;
use crate::cpu::Cpu;
use crate::mapper::NsfMapper;
use crate::nsf::{self, NsfInfo};
use crate::savestate::{MAGIC, MIN_VERSION, SaveState, VERSION};

pub struct Nes {
    pub cpu: Cpu,
    region: Region,
    battery: bool,
    /// RAM cheats forced at the start of every frame.
    ram_cheats: Vec<(u16, u8)>,
}

impl Nes {
    /// Build a machine from an iNES/NES 2.0 ROM image, or from an NSF music
    /// file (detected by its magic; see [`Nes::new_nsf`]).
    pub fn new(rom: &[u8]) -> Result<Self, String> {
        Self::new_with_region(rom, None)
    }

    /// Like [`Nes::new`], but `region` (when Some) overrides the TV system
    /// the ROM header declares - for Dendy-targeted games whose iNES headers
    /// can't say so, or PAL games dumped with an NTSC header.
    pub fn new_with_region(rom: &[u8], region: Option<Region>) -> Result<Self, String> {
        if nsf::is_nsf(rom) {
            return Self::new_nsf(rom);
        }
        let (cart, header_region, battery) = load_rom(rom)?;
        let region = region.unwrap_or(header_region);
        let mut bus = Bus::with_region(cart, region);
        // Vs. System cabinet: RGB PPU and the coin-op I/O on $4016/$4017.
        if let Some(ppu) = crate::vs::detect(rom) {
            bus.ppu.set_vs_ppu(Some(ppu));
            bus.vs = Some(crate::vs::VsIo::default());
        }
        let mut cpu = Cpu::new(bus);
        cpu.reset();
        Ok(Nes {
            cpu,
            region,
            battery,
            ram_cheats: Vec::new(),
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
            ram_cheats: Vec::new(),
        })
    }

    /// Whether `data` looks like a Famicom Disk System image (fwNES header
    /// or a headerless dump starting with the disk info block).
    pub fn is_fds_image(data: &[u8]) -> bool {
        data.starts_with(b"FDS\x1A")
            || (data.len() >= 15 && data[0] == 0x01 && &data[1..15] == b"*NINTENDO-HVC*")
    }

    /// Let the FDS pick the disk side the game asks for (default on).
    pub fn fds_set_auto_insert(&mut self, on: bool) {
        self.cpu.bus.cart.fds_set_auto_insert(on);
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
            ram_cheats: Vec::new(),
        })
    }

    /// Whether the cartridge has a speech chip (Jaleco uPD7755/uPD7756)
    /// whose recordings the host should supply with
    /// [`Nes::set_speech_samples`]; without them the speech is silent.
    pub fn wants_speech_samples(&self) -> bool {
        self.cpu.bus.cart.has_speech()
    }

    /// Supply the speech chip's recordings: mono clips in -1..1 at
    /// `sample_rate`, indexed by the chip's sample number (`None` = missing).
    /// Host data, not machine state: keep them across savestate loads by
    /// supplying them once after building the machine.
    pub fn set_speech_samples(&mut self, samples: Vec<Option<Vec<f32>>>, sample_rate: u32) {
        self.cpu.bus.cart.set_speech_samples(samples, sample_rate);
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

    /// Battery-backed save data to persist as a .sav file (PRG RAM, or the
    /// PRG flash of self-saving boards); None when the cartridge has no
    /// battery or nothing to save.
    pub fn battery_ram(&self) -> Option<&[u8]> {
        if self.battery {
            self.cpu.bus.cart.battery_save()
        } else {
            None
        }
    }

    /// Restore a previously saved .sav file. Ignored when the cartridge has
    /// no battery. PRG RAM takes the overlapping prefix of a size mismatch;
    /// flash boards ignore a mismatched file entirely.
    pub fn load_battery_ram(&mut self, data: &[u8]) {
        if !self.battery {
            return;
        }
        self.cpu.bus.cart.battery_restore(data);
    }

    /// Serialize the full machine state to a savestate blob. The state is
    /// only meaningful for the currently loaded ROM.
    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let state = self.capture(self.cpu.bus.cart.save_state());
        serde_json::to_vec(&state).map_err(|e| format!("failed to serialize savestate: {e}"))
    }

    fn capture(&self, mapper: Vec<u8>) -> SaveState {
        SaveState {
            magic: MAGIC,
            version: VERSION,
            region: self.region,
            cpu: self.cpu.save_state(),
            bus: self.cpu.bus.save_state(),
            ppu: self.cpu.bus.ppu.clone(),
            apu: self.cpu.bus.apu.save_state(),
            controller: self.cpu.bus.controller1.clone(),
            controller2: Some(self.cpu.bus.controller2.clone()),
            mapper,
        }
    }

    /// Restore a savestate blob produced by [`Nes::save_state`]. Rejects blobs
    /// with the wrong magic/version or a TV region that doesn't match the
    /// loaded ROM; mapper state from a different ROM is rejected by the mapper.
    /// Host audio configuration (sample rate, filters) is preserved.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        let state: SaveState =
            serde_json::from_slice(data).map_err(|e| format!("not a valid savestate: {e}"))?;
        let mapper = state.mapper.clone();
        self.apply(state, &mapper)
    }

    /// In-memory snapshot for rewind: like [`Nes::save_state`] but with the
    /// mapper JSON appended raw instead of re-encoded as a JSON byte array
    /// (a third of the size, and text that dedups well between frames).
    /// Layout: u32 LE length of the core JSON, core JSON, mapper JSON.
    pub fn snapshot(&self) -> Vec<u8> {
        let core = serde_json::to_vec(&self.capture(Vec::new())).expect("serialize snapshot");
        let mapper = self.cpu.bus.cart.save_state();
        let mut out = Vec::with_capacity(4 + core.len() + mapper.len());
        out.extend_from_slice(&(core.len() as u32).to_le_bytes());
        out.extend_from_slice(&core);
        out.extend_from_slice(&mapper);
        out
    }

    /// Restore a blob produced by [`Nes::snapshot`].
    pub fn restore_snapshot(&mut self, data: &[u8]) -> Result<(), String> {
        let bad = || "corrupt snapshot".to_string();
        let len = u32::from_le_bytes(data.get(..4).ok_or_else(bad)?.try_into().unwrap()) as usize;
        let core = data.get(4..4 + len).ok_or_else(bad)?;
        let state: SaveState = serde_json::from_slice(core).map_err(|e| e.to_string())?;
        self.apply(state, &data[4 + len..])
    }

    fn apply(&mut self, state: SaveState, mapper: &[u8]) -> Result<(), String> {
        if state.magic != MAGIC {
            return Err("not a savestate (bad magic)".into());
        }
        if !(MIN_VERSION..=VERSION).contains(&state.version) {
            return Err(format!(
                "unsupported savestate version {} (expected {VERSION})",
                state.version
            ));
        }
        if state.region != self.region {
            return Err("savestate is for a different TV region".into());
        }
        self.cpu.bus.cart.load_state(mapper)?;
        self.cpu.load_state(state.cpu);
        self.cpu.bus.load_state(state.bus);
        self.cpu.bus.ppu = state.ppu;
        self.cpu.bus.apu.load_state(state.apu);
        // Port wiring (Four Score) is host configuration: keep the live one.
        let wiring = (
            self.cpu.bus.controller1.wiring(),
            self.cpu.bus.controller2.wiring(),
        );
        self.cpu.bus.controller1 = state.controller;
        if let Some(c2) = state.controller2 {
            self.cpu.bus.controller2 = c2;
        }
        self.cpu.bus.controller1.set_four_score(wiring.0);
        self.cpu.bus.controller2.set_four_score(wiring.1);
        Ok(())
    }

    /// Run until the PPU enters vblank (one full frame).
    pub fn run_frame(&mut self) {
        // Poke directly rather than through Bus::write so cheats never
        // disturb open bus or the watch logs.
        for &(addr, value) in &self.ram_cheats {
            match addr {
                0x0000..=0x1FFF => self.cpu.bus.ram[(addr & 0x7FF) as usize] = value,
                _ => self.cpu.bus.cart.cpu_write(addr, value),
            }
        }
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

    /// Whether the loaded ROM is a Vs. System arcade dump.
    pub fn is_vs_system(&self) -> bool {
        self.cpu.bus.vs.is_some()
    }

    /// Vs. System coin slots and service button, as held levels. The host
    /// pulses a coin for a few frames per insert. Ignored off Vs.
    pub fn set_vs_inputs(&mut self, coin1: bool, coin2: bool, service: bool) {
        if let Some(vs) = &mut self.cpu.bus.vs {
            vs.coin1 = coin1;
            vs.coin2 = coin2;
            vs.service = service;
        }
    }

    /// Vs. System DIP switches 1-8 (bit 0 = switch 1). Ignored off Vs.
    pub fn set_vs_dip(&mut self, dip: u8) {
        if let Some(vs) = &mut self.cpu.bus.vs {
            vs.dip = dip;
        }
    }

    /// Choose what is plugged into the controller ports.
    pub fn set_input_device(&mut self, device: InputDevice) {
        self.cpu.bus.set_input_device(device);
    }

    /// Button masks for players 1-4. Players 3 and 4 are only visible to
    /// the game through a Four Score.
    pub fn set_buttons(&mut self, masks: [u8; 4]) {
        let bus = &mut self.cpu.bus;
        bus.controller1.state = masks[0];
        bus.controller2.state = masks[1];
        bus.controller1.state_hi = masks[2];
        bus.controller2.state_hi = masks[3];
    }

    /// Aim and trigger of the Zapper (ignored unless it is plugged in).
    /// Coordinates are NES pixels; `None` aims off screen.
    pub fn set_zapper(&mut self, aim: Option<(i32, i32)>, trigger: bool) {
        if let Some(z) = &mut self.cpu.bus.zapper {
            (z.x, z.y) = aim.unwrap_or((-1, -1));
            z.trigger = trigger;
        }
    }

    /// Replace the active cheats. ROM patches take effect on the next read;
    /// RAM values are forced at the start of every frame.
    pub fn set_cheats(&mut self, cheats: &[Cheat]) {
        self.cpu.bus.rom_patches.clear();
        self.ram_cheats.clear();
        for &c in cheats {
            match c {
                Cheat::Rom {
                    addr,
                    value,
                    compare,
                } => self.cpu.bus.rom_patches.push(RomPatch {
                    addr,
                    value,
                    compare,
                }),
                Cheat::Ram { addr, value } => self.ram_cheats.push((addr, value)),
            }
        }
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
    fn v1_savestates_still_load_and_v2_is_smaller() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nestest.nes");
        let data = std::fs::read(path).unwrap();
        let mut nes = Nes::new(&data).unwrap();
        for _ in 0..10 {
            nes.run_frame();
        }
        let v2 = nes.save_state().unwrap();
        // Rebuild the same state in the v1 layout: mapper JSON as a byte array.
        let mut doc: serde_json::Value = serde_json::from_slice(&v2).unwrap();
        let mapper_bytes = serde_json::to_vec(&doc["mapper"]).unwrap();
        doc["mapper"] = serde_json::Value::from(mapper_bytes);
        doc["version"] = 1.into();
        let v1 = serde_json::to_vec(&doc).unwrap();
        assert!(v2.len() < v1.len(), "v2 {} vs v1 {}", v2.len(), v1.len());

        for _ in 0..20 {
            nes.run_frame();
        }
        let reference = nes.framebuffer().to_vec();
        for blob in [&v1, &v2] {
            let mut m = Nes::new(&data).unwrap();
            m.load_state(blob).unwrap();
            for _ in 0..20 {
                m.run_frame();
            }
            assert_eq!(m.framebuffer(), reference.as_slice());
        }
    }

    #[test]
    fn chr_rom_stays_out_of_savestates_and_old_states_still_load() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nestest.nes");
        let data = std::fs::read(path).unwrap();
        let mut nes = Nes::new(&data).unwrap();
        for _ in 0..10 {
            nes.run_frame();
        }
        let blob = nes.save_state().unwrap();
        let mut doc: serde_json::Value = serde_json::from_slice(&blob).unwrap();
        assert!(
            doc["mapper"]["chr"].is_null(),
            "CHR ROM must not be serialized"
        );
        assert!(blob.len() < 30_000, "state is {} bytes", blob.len());

        for _ in 0..20 {
            nes.run_frame();
        }
        let reference = nes.framebuffer().to_vec();

        // The same state in the old layout: CHR ROM embedded as an array.
        let prg_len = data[4] as usize * 0x4000;
        let chr_rom = data[16 + prg_len..16 + prg_len + data[5] as usize * 0x2000].to_vec();
        let mut old = doc.clone();
        old["mapper"]["chr"] = serde_json::Value::from(chr_rom.clone());
        let old = serde_json::to_vec(&old).unwrap();

        for blob in [&blob, &old] {
            let mut m = Nes::new(&data).unwrap();
            m.load_state(blob).unwrap();
            for _ in 0..20 {
                m.run_frame();
            }
            assert_eq!(m.framebuffer(), reference.as_slice());
            // Re-saving drops the redundant CHR ROM again.
            let again: serde_json::Value =
                serde_json::from_slice(&m.save_state().unwrap()).unwrap();
            assert!(again["mapper"]["chr"].is_null());
        }

        // A CHR array of the wrong size is a different ROM: rejected.
        doc["mapper"]["chr"] = serde_json::Value::from(vec![0u8; 0x1000]);
        let wrong = serde_json::to_vec(&doc).unwrap();
        assert!(Nes::new(&data).unwrap().load_state(&wrong).is_err());
    }

    #[test]
    fn chr_ram_survives_savestates() {
        // NROM with CHR RAM (CHR size 0): pattern writes must be kept.
        let mut rom = rom(0x00);
        rom[5] = 0;
        rom.truncate(16 + 32 * 1024);
        let mut nes = Nes::new(&rom).unwrap();
        nes.cpu.bus.cart.ppu_write(0x0123, 0x5A);
        let blob = nes.save_state().unwrap();
        let mut m = Nes::new(&rom).unwrap();
        m.load_state(&blob).unwrap();
        assert_eq!(m.cpu.bus.cart.ppu_read(0x0123), 0x5A);
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
    fn snapshot_roundtrip_matches_savestate() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nestest.nes");
        let data = std::fs::read(path).unwrap();
        let mut nes = Nes::new(&data).unwrap();
        for _ in 0..20 {
            nes.run_frame();
        }
        let snap = nes.snapshot();
        for _ in 0..30 {
            nes.run_frame();
        }
        let reference = nes.framebuffer().to_vec();
        let mut restored = Nes::new(&data).unwrap();
        restored.restore_snapshot(&snap).unwrap();
        for _ in 0..30 {
            restored.run_frame();
        }
        assert_eq!(restored.framebuffer(), reference.as_slice());
        assert!(restored.restore_snapshot(&snap[..3]).is_err());
    }

    /// NROM image whose PRG is filled with `fill` and whose reset vector
    /// points at $8000.
    fn nrom_filled(fill: u8) -> Vec<u8> {
        let mut data = rom(0x00);
        data[16..16 + 0x8000].fill(fill);
        data[16 + 0x7FFC] = 0x00;
        data[16 + 0x7FFD] = 0x80;
        data
    }

    #[test]
    fn region_override_and_dendy_timing() {
        let data = nrom_filled(0xEA);
        let mut nes = Nes::new_with_region(&data, Some(Region::Dendy)).unwrap();
        assert_eq!(nes.region(), Region::Dendy);
        // A Dendy frame is 312 lines of 341 dots at 3 dots per CPU cycle.
        nes.run_frame();
        let c0 = nes.cpu.bus.cycles;
        nes.run_frame();
        let cycles = nes.cpu.bus.cycles - c0;
        assert!((35_460..=35_470).contains(&cycles), "{cycles} CPU cycles");
        // NES 2.0 timing byte 3 selects Dendy on its own.
        let mut nes2 = data.clone();
        nes2[7] = 0x08;
        nes2[12] = 3;
        assert_eq!(Nes::new(&nes2).unwrap().region(), Region::Dendy);
    }

    /// Vs. UniSystem (mapper 99) image: 32KB PRG, 16KB CHR, iNES Vs. bit.
    fn vs_rom() -> Vec<u8> {
        let mut data = vec![0u8; 16 + 0x8000 + 0x4000];
        data[0..4].copy_from_slice(b"NES");
        data[4] = 2;
        data[5] = 2;
        data[6] = 0x38; // mapper 99 low nibble 3, four-screen
        data[7] = 0x61; // mapper 99 high nibble 6, Vs. bit
        data[16..16 + 0x8000].fill(0xEA);
        data[16 + 0x7FFC] = 0x00;
        data[16 + 0x7FFD] = 0x80;
        // CHR bank 0 = $00, bank 1 = $11.
        data[16 + 0x8000 + 0x2000..].fill(0x11);
        data
    }

    #[test]
    fn vs_system_io_ports_and_mapper_99() {
        let mut nes = Nes::new(&vs_rom()).unwrap();
        assert!(nes.is_vs_system());
        nes.set_vs_dip(0b1100_0101);
        nes.set_vs_inputs(true, false, true);
        let bus = &mut nes.cpu.bus;
        let p1 = bus.read(0x4016);
        assert_eq!(p1 & 0x7E, 0x04 | (0b01 << 3) | 0x20, "service, DIP1, coin1");
        assert_eq!(bus.read(0x4017) & 0xFC, 0b1100_0100, "DIP 3-8");
        // $4016 bit 2 selects CHR bank 1.
        assert_eq!(bus.cart.ppu_read(0x0000), 0x00);
        bus.write(0x4016, 0x04);
        assert_eq!(bus.cart.ppu_read(0x0000), 0x11);
        // Home-console ROMs see none of it.
        let mut home = Nes::new(&nrom_filled(0xEA)).unwrap();
        assert!(!home.is_vs_system());
        home.set_vs_inputs(true, true, true); // no-op
        assert!(home.cpu.bus.vs.is_none());
    }

    #[test]
    fn rc2c05_swaps_ctrl_and_mask_and_reports_its_id() {
        let mut data = vs_rom();
        data[7] = 0x69; // NES 2.0, console type Vs.
        data[6] = 0x38;
        data[13] = 0x09; // RC2C05-02
        let mut nes = Nes::new(&data).unwrap();
        let bus = &mut nes.cpu.bus;
        assert_eq!(bus.read(0x2002) & 0x3F, 0x3D);
        // A write to $2001 lands in PPUCTRL: NMI enable is bit 7.
        bus.write(0x2001, 0x80);
        assert!(bus.ppu.ctrl & 0x80 != 0);
    }

    #[test]
    fn rom_cheats_patch_reads() {
        let mut nes = Nes::new(&nrom_filled(0xEA)).unwrap();
        nes.set_cheats(&[
            crate::cheats::parse("8123:42").unwrap(),
            crate::cheats::parse("8124?EA:43").unwrap(),
            crate::cheats::parse("8125?00:44").unwrap(), // compare fails
        ]);
        assert_eq!(nes.cpu.bus.read(0x8123), 0x42);
        assert_eq!(nes.cpu.bus.read(0x8124), 0x43);
        assert_eq!(nes.cpu.bus.read(0x8125), 0xEA);
        nes.set_cheats(&[]);
        assert_eq!(nes.cpu.bus.read(0x8123), 0xEA);
    }

    #[test]
    fn ram_cheats_are_forced_each_frame() {
        // PRG of NOPs: the CPU never touches $0300 itself.
        let mut nes = Nes::new(&nrom_filled(0xEA)).unwrap();
        nes.set_cheats(&[crate::cheats::parse("0300:99").unwrap()]);
        nes.run_frame();
        assert_eq!(nes.cpu.bus.ram[0x300], 0x99);
        nes.cpu.bus.ram[0x300] = 0;
        nes.run_frame();
        assert_eq!(nes.cpu.bus.ram[0x300], 0x99);
    }

    #[test]
    fn four_score_wiring_survives_state_load() {
        let mut nes = Nes::new(&nrom_filled(0xEA)).unwrap();
        let blob = nes.save_state().unwrap();
        nes.set_input_device(InputDevice::FourScore);
        nes.set_buttons([0, 0, crate::controller::BTN_A, 0]);
        nes.load_state(&blob).unwrap();
        nes.set_buttons([0, 0, crate::controller::BTN_A, 0]);
        let bus = &mut nes.cpu.bus;
        bus.write(0x4016, 1);
        bus.controller1.clock_put_cycle();
        bus.write(0x4016, 0);
        let bits: Vec<u8> = (0..9).map(|_| bus.read(0x4016) & 1).collect();
        assert_eq!(bits[8], 1, "P3 A must follow P1's 8 bits");
    }

    #[test]
    fn zapper_answers_port_two() {
        let mut nes = Nes::new(&nrom_filled(0xEA)).unwrap();
        nes.set_input_device(InputDevice::Zapper);
        nes.set_zapper(Some((10, 10)), true);
        assert_eq!(nes.cpu.bus.read(0x4017) & 0x18, 0x18); // trigger, dark
        nes.set_input_device(InputDevice::Standard);
        assert_eq!(nes.cpu.bus.read(0x4017) & 0x18, 0x00);
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
