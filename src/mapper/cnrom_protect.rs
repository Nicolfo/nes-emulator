use super::discrete::{bank_index, chr_or_ram};
use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Mapper 185: CNROM whose CHR ROM is gated by a copy-protection diode
/// network. Writes to $8000-$FFFF don't switch banks; they enable or disable
/// the CHR ROM, and games check that pattern reads come back as garbage with
/// the "wrong" value latched. NES 2.0 submappers 4-7 say which value of the
/// latch's low two bits enables CHR (0-3). Without a submapper the classic
/// heuristic applies: CHR is enabled when `value & $0F != 0` and
/// `value != $13` (Spy vs Spy, B-Wings, Bird Week, Mighty Bomb Jack).
/// Disabled CHR reads return $FF. CHR starts enabled at power-on.
#[derive(Serialize, Deserialize)]
pub struct CnromProtect {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    submapper: u8,
    chr_enabled: bool,
    mirroring: Mirroring,
}

impl CnromProtect {
    pub fn new(submapper: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x2000);
        CnromProtect {
            prg,
            chr,
            chr_is_ram,
            submapper,
            chr_enabled: true,
            mirroring,
        }
    }

    fn enables(&self, val: u8) -> bool {
        match self.submapper {
            4..=7 => val & 3 == self.submapper - 4,
            _ => val & 0x0F != 0 && val != 0x13,
        }
    }
}

impl Mapper for CnromProtect {
    crate::impl_mapper_savestate!(chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[bank_index(self.prg.len(), 0x8000, 0, addr as usize)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            self.chr_enabled = self.enables(val);
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        if !self.chr_enabled {
            return 0xFF;
        }
        self.chr[bank_index(self.chr.len(), 0x2000, 0, addr as usize)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram && self.chr_enabled {
            let i = bank_index(self.chr.len(), 0x2000, 0, addr as usize);
            self.chr[i] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(submapper: u8) -> CnromProtect {
        CnromProtect::new(
            submapper,
            vec![0; 0x8000],
            vec![0x5A; 0x2000],
            Mirroring::Vertical,
        )
    }

    #[test]
    fn heuristic_gates_chr() {
        let mut m = board(0);
        assert_eq!(m.ppu_read(0x10), 0x5A, "enabled at power-on");
        m.cpu_write(0x8000, 0x00);
        assert_eq!(m.ppu_read(0x10), 0xFF);
        m.cpu_write(0x8000, 0x13);
        assert_eq!(m.ppu_read(0x10), 0xFF);
        m.cpu_write(0x8000, 0x21);
        assert_eq!(m.ppu_read(0x10), 0x5A);
    }

    #[test]
    fn submapper_picks_enable_value() {
        let mut m = board(6);
        m.cpu_write(0x8000, 0x01);
        assert_eq!(m.ppu_read(0), 0xFF);
        m.cpu_write(0x8000, 0x02);
        assert_eq!(m.ppu_read(0), 0x5A);
    }
}
