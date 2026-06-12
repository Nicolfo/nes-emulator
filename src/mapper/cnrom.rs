use super::{Mapper, Mirroring};

/// CNROM (mapper 3): fixed PRG like NROM, one 8KB switchable CHR bank.
/// Real boards have bus conflicts (written value ANDed with ROM byte);
/// not emulated.
pub struct Cnrom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Cnrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        Cnrom {
            prg,
            chr,
            chr_bank: 0,
            mirroring,
        }
    }
}

impl Mapper for Cnrom {
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
        let banks = self.chr.len() / 0x2000;
        self.chr[(self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)]
    }

    fn ppu_write(&mut self, _addr: u16, _val: u8) {}

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
    fn prg_mirrors_16k() {
        let mut m = cnrom();
        assert_eq!(m.cpu_read(0x8123), m.cpu_read(0xC123));
    }
}
