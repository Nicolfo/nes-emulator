use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// CNROM (mapper 3): fixed PRG like NROM, one 8KB switchable CHR bank.
/// Real boards have bus conflicts (written value ANDed with ROM byte);
/// not emulated. A header declaring no CHR ROM gets 8KB of CHR RAM (the
/// iNES convention), keeping such images from crashing the bank math.
#[derive(Serialize, Deserialize)]
pub struct Cnrom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    #[serde(default)]
    chr_is_ram: bool,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Cnrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Cnrom {
            prg,
            chr,
            chr_is_ram,
            chr_bank: 0,
            mirroring,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x2000;
        (self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)
    }
}

impl Mapper for Cnrom {
    crate::impl_mapper_savestate!(prg, chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            // mask handles both 16KB (mirrored) and 32KB PRG
            self.prg[(addr as usize - 0x8000) & (self.prg.len() - 1)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            self.chr_bank = val;
        }
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

    fn cnrom() -> Cnrom {
        // 16KB PRG (mirrored), 4 CHR banks; each CHR byte = its bank index.
        let prg: Vec<u8> = (0..0x4000).map(|i| (i & 0xFF) as u8).collect();
        let chr: Vec<u8> = (0..4 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        Cnrom::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn chr_bank_switch() {
        let mut m = cnrom();
        assert_eq!(m.ppu_read(0x0000), 0);
        m.cpu_write(0x8000, 2);
        assert_eq!(m.ppu_read(0x0000), 2);
        // Bank index wraps modulo bank count.
        m.cpu_write(0x8000, 5);
        assert_eq!(m.ppu_read(0x0000), 1);
    }

    #[test]
    fn chr_ram_fallback_rw() {
        let mut m = Cnrom::new(vec![0; 0x4000], vec![], Mirroring::Vertical);
        m.ppu_write(0x0123, 0x42);
        assert_eq!(m.ppu_read(0x0123), 0x42);
    }

    #[test]
    fn prg_mirrors_16k() {
        let mut m = cnrom();
        assert_eq!(m.cpu_read(0x8123), m.cpu_read(0xC123));
    }
}
