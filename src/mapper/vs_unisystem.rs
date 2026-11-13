use super::Chr;
use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Vs. UniSystem discrete board (mapper 99: Vs. Super Mario Bros., Vs.
/// Excitebike, Vs. Golf...). Bit 2 of a $4016 write - the same write that
/// strobes the controllers - selects the 8KB CHR bank and, on the 40KB
/// Vs. Gumshoe, the 8KB PRG bank at $8000 (bank 0 or 4; $A000-$FFFF holds
/// banks 1-3). $6000-$7FFF is the cabinet's 2KB work RAM, mirrored.
///
/// On a Vs. DualSystem the two boards share that RAM, one at a time: the
/// board without access reads open bus there and its writes are lost.
/// [`crate::nes::Nes`] hands the RAM over by swapping the two buffers.
#[derive(Serialize, Deserialize)]
pub struct VsUnisystem {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Chr,
    #[serde(default)]
    chr_is_ram: bool,
    ram: Vec<u8>,
    bank_bit: bool,
    mirroring: Mirroring,
    /// DualSystem: this board currently owns the shared work RAM.
    #[serde(default = "owns_ram")]
    ram_access: bool,
}

fn owns_ram() -> bool {
    true
}

impl VsUnisystem {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        VsUnisystem {
            prg,
            chr: chr.into(),
            chr_is_ram,
            ram: vec![0; 0x800],
            bank_bit: false,
            mirroring,
            ram_access: true,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = (self.chr.len() / 0x2000).max(1);
        (self.bank_bit as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)
    }

    fn prg_offset(&self, addr: u16) -> usize {
        let banks = (self.prg.len() / 0x2000).max(1);
        let slot = (addr as usize - 0x8000) >> 13;
        // Only a 40KB+ image has a bank 4 to swap in.
        let bank = if slot == 0 && self.bank_bit && banks > 4 {
            4
        } else {
            slot
        };
        (bank % banks) * 0x2000 + (addr as usize & 0x1FFF)
    }
}

impl Mapper for VsUnisystem {
    crate::impl_mapper_savestate!(chr, ram);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram].into();
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if (0x6000..0x8000).contains(&addr) && self.ram_access {
            self.ram[addr as usize & 0x7FF] = val;
        }
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        self.ram_access.then(|| self.ram[addr as usize & 0x7FF])
    }

    fn battery_save(&self) -> Option<&[u8]> {
        self.ram_access.then_some(&self.ram[..])
    }

    fn battery_restore(&mut self, data: &[u8]) {
        if self.ram_access {
            let n = self.ram.len().min(data.len());
            self.ram[..n].copy_from_slice(&data[..n]);
        }
    }

    fn vs_ram_access(&self) -> bool {
        self.ram_access
    }

    fn vs_set_ram_access(&mut self, on: bool) {
        self.ram_access = on;
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.ram)
    }

    fn vs_io_write(&mut self, val: u8) {
        self.bank_bit = val & 0x04 != 0;
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[self.chr_offset(addr)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram {
            let off = self.chr_offset(addr);
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PRG and CHR filled with their 8KB bank index.
    fn board(prg_banks: usize, chr_banks: usize) -> VsUnisystem {
        let prg = (0..prg_banks * 0x2000)
            .map(|i| (i / 0x2000) as u8)
            .collect();
        let chr = (0..chr_banks * 0x2000)
            .map(|i| (i / 0x2000) as u8)
            .collect();
        VsUnisystem::new(prg, chr, Mirroring::FourScreen)
    }

    #[test]
    fn chr_bank_follows_4016_bit_2() {
        let mut m = board(4, 2);
        assert_eq!(m.ppu_read(0x0123), 0);
        m.vs_io_write(0x04);
        assert_eq!(m.ppu_read(0x0123), 1);
        m.vs_io_write(0x01); // strobe only: bank bit clear
        assert_eq!(m.ppu_read(0x1FFF), 0);
        // 32KB PRG is fixed regardless of the bank bit.
        m.vs_io_write(0x04);
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xE000), 3);
    }

    #[test]
    fn gumshoe_40kb_swaps_prg_at_8000() {
        // 40KB arrives padded to 48KB by the loader.
        let mut m = board(6, 2);
        assert_eq!(m.cpu_read(0x8000), 0);
        m.vs_io_write(0x04);
        assert_eq!(m.cpu_read(0x8000), 4);
        assert_eq!(m.cpu_read(0xA000), 1);
        assert_eq!(m.cpu_read(0xFFFF), 3);
    }

    #[test]
    fn work_ram_is_2kb_mirrored() {
        let mut m = board(4, 1);
        m.cpu_write(0x6001, 0x5A);
        assert_eq!(m.prg_ram_read(0x6801), Some(0x5A));
        assert_eq!(m.prg_ram_read(0x7801), Some(0x5A));
        // A single CHR bank tolerates the bank bit.
        m.vs_io_write(0x04);
        assert_eq!(m.ppu_read(0x0000), 0);
    }

    #[test]
    fn dualsystem_board_without_ram_access_sees_open_bus() {
        let mut m = board(4, 1);
        m.cpu_write(0x6000, 0x42);
        m.vs_set_ram_access(false);
        assert!(!m.vs_ram_access());
        assert_eq!(m.prg_ram_read(0x6000), None);
        m.cpu_write(0x6000, 0x99); // lost
        assert!(m.battery_save().is_none());
        m.vs_set_ram_access(true);
        assert_eq!(m.prg_ram_read(0x6000), Some(0x42));
        // The flag survives a savestate.
        let s = m.save_state();
        m.vs_set_ram_access(false);
        m.load_state(&s).unwrap();
        assert!(m.vs_ram_access());
    }
}
