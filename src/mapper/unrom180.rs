use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// UNROM 180 (mapper 180, used by Crazy Climber): UxROM with the banking
/// inverted. The FIXED bank moves and the switchable one doesn't: $8000-$BFFF
/// is always PRG bank 0, while $C000-$FFFF selects `prg_bank`. 8KB CHR (RAM).
/// Real boards rely on bus conflicts (written value ANDed with ROM byte) which
/// Crazy Climber depends on; not emulated (plain register write).
#[derive(Serialize, Deserialize)]
pub struct Unrom180 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_bank: u8,
    mirroring: Mirroring,
}

impl Unrom180 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Unrom180 {
            prg,
            chr,
            chr_is_ram,
            prg_bank: 0,
            mirroring,
        }
    }
}

impl Mapper for Unrom180 {
    crate::impl_mapper_savestate!();
    fn cpu_read(&mut self, addr: u16) -> u8 {
        let banks = self.prg.len() / 0x4000;
        match addr {
            // Fixed: always bank 0, never moves.
            0x8000..=0xBFFF => self.prg[addr as usize & 0x3FFF],
            // Switchable: the moving bank lives in the high half.
            0xC000..=0xFFFF => {
                self.prg[(self.prg_bank as usize % banks) * 0x4000 + (addr as usize & 0x3FFF)]
            }
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            self.prg_bank = val;
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[(addr as usize) & 0x1FFF]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram {
            self.chr[(addr as usize) & 0x1FFF] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unrom180() -> Unrom180 {
        // 8 PRG banks (128KB), CHR RAM; each PRG byte = its bank index.
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        Unrom180::new(prg, vec![], Mirroring::Vertical)
    }

    #[test]
    fn fixed_8000_switchable_c000() {
        let mut m = unrom180();
        // $8000 is fixed bank 0; $C000 defaults to switchable bank 0.
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xC000), 0);
        m.cpu_write(0x8000, 3);
        // $8000 must NOT move; $C000 follows the register.
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xC000), 3);
    }

    #[test]
    fn bank_wraps_modulo() {
        let mut m = unrom180();
        m.cpu_write(0x8000, 9);
        assert_eq!(m.cpu_read(0xC000), 1);
        // Fixed half is unaffected by the wrap.
        assert_eq!(m.cpu_read(0x8000), 0);
    }

    #[test]
    fn chr_ram_rw() {
        let mut m = unrom180();
        m.ppu_write(0x1234, 0xAB);
        assert_eq!(m.ppu_read(0x1234), 0xAB);
    }
}
