use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Jaleco JF-17 / JF-19 (mapper 72). A single write port at $8000-$FFFF where
/// two trigger bits decide whether the low nibble updates the PRG or CHR bank:
///
/// ```text
/// 7  bit  0
/// P C .. BBBB
/// | |    |
/// | |    +--- bits 0-3: bank number
/// | +-------- bit 6: when set, load CHR bank (8KB @ $0000)
/// +---------- bit 7: when set, load PRG bank (16KB @ $8000)
/// ```
/// PRG: $8000-$BFFF switchable, $C000-$FFFF fixed to the last 16KB. Mirroring
/// is hardwired. The JF-19 adds a uPD7756C sample player, which is not emulated
/// (no game depends on it for gameplay). Games: Pinball Quest, Moero!! Pro
/// Yakyuu, Moero!! Pro Tennis.
#[derive(Serialize, Deserialize)]
pub struct JalecoJf17 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl JalecoJf17 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        JalecoJf17 {
            prg,
            chr,
            chr_is_ram,
            prg_bank: 0,
            chr_bank: 0,
            mirroring,
        }
    }
}

impl Mapper for JalecoJf17 {
    crate::impl_mapper_savestate!(prg, chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        let banks = self.prg.len() / 0x4000;
        match addr {
            0x8000..=0xBFFF => {
                self.prg[(self.prg_bank as usize % banks) * 0x4000 + (addr as usize & 0x3FFF)]
            }
            0xC000..=0xFFFF => self.prg[(banks - 1) * 0x4000 + (addr as usize & 0x3FFF)],
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            // The two trigger bits gate which bank the low nibble updates; a
            // write with neither set latches nothing.
            if val & 0x80 != 0 {
                self.prg_bank = val & 0x0F;
            }
            if val & 0x40 != 0 {
                self.chr_bank = val & 0x0F;
            }
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let banks = self.chr.len() / 0x2000;
        self.chr[(self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)]
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

    fn jf17() -> JalecoJf17 {
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        JalecoJf17::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_latch() {
        let mut m = jf17();
        m.cpu_write(0x8000, 0x83); // PRG trigger + bank 3
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xC000), 7); // last fixed
    }

    #[test]
    fn chr_latch() {
        let mut m = jf17();
        m.cpu_write(0x8000, 0x45); // CHR trigger + bank 5
        assert_eq!(m.ppu_read(0x0000), 5);
    }

    #[test]
    fn no_trigger_latches_nothing() {
        let mut m = jf17();
        m.cpu_write(0x8000, 0x83); // PRG bank 3
        m.cpu_write(0x8000, 0x05); // no trigger bits set
        assert_eq!(m.cpu_read(0x8000), 3); // unchanged
    }

    #[test]
    fn both_triggers_at_once() {
        let mut m = jf17();
        m.cpu_write(0x8000, 0xC2); // PRG + CHR trigger, bank 2
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.ppu_read(0x0000), 2);
    }
}
