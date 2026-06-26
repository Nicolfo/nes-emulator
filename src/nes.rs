use crate::bus::Bus;
use crate::cartridge::{Region, load_rom};
use crate::cpu::Cpu;
use crate::savestate::{MAGIC, SaveState, VERSION};

pub struct Nes {
    pub cpu: Cpu,
    region: Region,
    battery: bool,
}

impl Nes {
    pub fn new(rom: &[u8]) -> Result<Self, String> {
        let (cart, region, battery) = load_rom(rom)?;
        let mut cpu = Cpu::new(Bus::with_region(cart, region));
        cpu.reset();
        Ok(Nes {
            cpu,
            region,
            battery,
        })
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
            controller1: self.cpu.bus.controller1.clone(),
            controller2: self.cpu.bus.controller2.clone(),
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
        // Validate untrusted invariants before mutating anything, so a
        // malformed blob returns Err instead of corrupting the live machine
        // (or panicking later on an out-of-range index).
        state.ppu.validate()?;
        self.cpu.bus.cart.load_state(&state.mapper)?;
        self.cpu.load_state(state.cpu);
        self.cpu.bus.load_state(state.bus);
        self.cpu.bus.ppu = state.ppu;
        self.cpu.bus.apu.load_state(state.apu);
        self.cpu.bus.controller1 = state.controller1;
        self.cpu.bus.controller2 = state.controller2;
        Ok(())
    }

    /// Run until the PPU enters vblank (one full frame).
    ///
    /// A real frame is ~29,781 (NTSC) / ~33,248 (PAL) CPU cycles. The loop is
    /// bounded well above that so a jammed CPU (a `JAM`/`KIL` opcode in corrupt
    /// PRG) or any pathological state that never reaches vblank bails out
    /// instead of spinning forever - the frontend stays responsive.
    pub fn run_frame(&mut self) {
        const MAX_FRAME_CYCLES: u64 = 100_000;
        self.cpu.bus.ppu.frame_complete = false;
        let mut budget: u64 = 0;
        while !self.cpu.bus.ppu.frame_complete {
            budget += self.cpu.step();
            if budget > MAX_FRAME_CYCLES {
                break;
            }
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

    /// A `state.version` from an older format (which embedded the ROM and had a
    /// single controller) must be rejected, not silently misread.
    #[test]
    fn load_state_rejects_old_version() {
        let mut nes = Nes::new(&rom(0x10)).unwrap();
        let blob = nes.save_state().unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&blob).unwrap();
        v["version"] = serde_json::json!(VERSION - 1);
        let tampered = serde_json::to_vec(&v).unwrap();
        assert!(nes.load_state(&tampered).is_err());
    }

    /// The savestate no longer embeds PRG/CHR ROM. nestest is a 24KB ROM whose
    /// PRG+CHR would balloon to ~80KB as a JSON byte array if embedded; the
    /// whole state stays well under that.
    #[test]
    fn savestate_does_not_embed_rom() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nestest.nes");
        let data = std::fs::read(path).unwrap();
        let mut nes = Nes::new(&data).unwrap();
        for _ in 0..20 {
            nes.run_frame();
        }
        let blob = nes.save_state().unwrap();
        assert!(
            blob.len() < 50_000,
            "savestate is {} bytes - ROM appears to be embedded",
            blob.len()
        );
    }

    /// A malformed savestate that claims more sprites than the fixed 8-entry
    /// sprite array must return an error, not panic on the next scanline.
    #[test]
    fn load_state_rejects_oversized_sprite_count() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nestest.nes");
        let data = std::fs::read(path).unwrap();
        let mut nes = Nes::new(&data).unwrap();
        nes.run_frame();
        let blob = nes.save_state().unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&blob).unwrap();
        v["ppu"]["sprite_count"] = serde_json::json!(99);
        let tampered = serde_json::to_vec(&v).unwrap();
        assert!(nes.load_state(&tampered).is_err());
        // The rejected restore left the machine runnable (no partial mutation).
        nes.run_frame();
    }

    /// A malformed mapper blob with empty CHR (the classic `len()/bank - 1`
    /// panic source) must be rejected rather than crashing on the first fetch.
    #[test]
    fn load_state_rejects_short_mapper_chr() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nestest.nes");
        let data = std::fs::read(path).unwrap();
        let mut nes = Nes::new(&data).unwrap();
        nes.run_frame();
        let blob = nes.save_state().unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&blob).unwrap();
        // Decode the inner mapper JSON, mark CHR as RAM but leave it empty, and
        // re-encode: the restore must catch the size mismatch.
        let inner_bytes: Vec<u8> = serde_json::from_value(v["mapper"].clone()).unwrap();
        let mut inner: serde_json::Value = serde_json::from_slice(&inner_bytes).unwrap();
        inner["chr_is_ram"] = serde_json::json!(true);
        inner["chr"] = serde_json::json!([]);
        v["mapper"] = serde_json::json!(serde_json::to_vec(&inner).unwrap());
        let tampered = serde_json::to_vec(&v).unwrap();
        assert!(nes.load_state(&tampered).is_err());
        nes.run_frame();
    }

    /// CHR-RAM contents are genuine state and must round-trip through a
    /// savestate. Unlike CHR-ROM (re-injected from the cartridge), the RAM
    /// image is serialized, so a divergent live machine restores the saved
    /// bytes rather than its current ones.
    #[test]
    fn savestate_roundtrips_chr_ram() {
        // mapper 0, 32KB PRG, no CHR ROM -> 8KB CHR-RAM.
        let mut data = vec![0u8; 16 + 32 * 1024];
        data[0..4].copy_from_slice(b"NES\x1A");
        data[4] = 2; // 32KB PRG
        data[5] = 0; // no CHR -> CHR-RAM
        let mut nes = Nes::new(&data).unwrap();
        nes.cpu.bus.cart.ppu_write(0x0123, 0xAB);
        assert_eq!(nes.cpu.bus.cart.ppu_read(0x0123), 0xAB);
        let blob = nes.save_state().unwrap();
        // Diverge after the snapshot.
        nes.cpu.bus.cart.ppu_write(0x0123, 0x5C);
        nes.load_state(&blob).unwrap();
        assert_eq!(nes.cpu.bus.cart.ppu_read(0x0123), 0xAB);
    }

    /// Two-player input must survive a savestate round-trip (controller2 was
    /// previously dropped from the snapshot).
    #[test]
    fn savestate_roundtrips_controller2() {
        use crate::controller::{BTN_A, BTN_START, Controller};
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nestest.nes");
        let data = std::fs::read(path).unwrap();
        let mut nes = Nes::new(&data).unwrap();
        nes.cpu.bus.controller2.set_button(BTN_A, true);
        nes.cpu.bus.controller2.set_button(BTN_START, true);
        // Strobe to latch the buttons into the shift register, then freeze.
        nes.cpu.bus.controller2.write(1);
        nes.cpu.bus.controller2.clock_put_cycle();
        nes.cpu.bus.controller2.write(0);
        let blob = nes.save_state().unwrap();
        // Diverge: a fresh controller forgets the latched buttons.
        nes.cpu.bus.controller2 = Controller::default();
        nes.load_state(&blob).unwrap();
        let p2: Vec<u8> = (0..4).map(|_| nes.cpu.bus.controller2.read() & 1).collect();
        assert_eq!(p2, vec![1, 0, 0, 1], "A, B, Select, Start for player 2");
    }
}
