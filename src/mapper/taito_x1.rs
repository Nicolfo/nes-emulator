use super::{Mapper, Mirroring, NtTarget};
use serde::{Deserialize, Serialize};

/// CHR offset for the Taito X1 layout: two 2KB banks and four 1KB banks,
/// every register holding a 1KB bank number (the 2KB ones ignore bit 0).
/// `invert` swaps the 2KB and 1KB halves (X1-017 only).
fn x1_chr_offset(chr_len: usize, regs: &[u8; 6], invert: bool, addr: u16) -> usize {
    let a = addr as usize & 0x1FFF;
    let a = if invert { a ^ 0x1000 } else { a };
    let bank = if a < 0x1000 {
        // 2KB slot: 1KB bank number with bit 0 replaced by the address.
        (regs[a >> 11] & 0x7E) as usize | ((a >> 10) & 1)
    } else {
        regs[2 + ((a - 0x1000) >> 10)] as usize
    };
    let banks = chr_len / 0x400;
    (bank % banks) * 0x400 + (addr as usize & 0x3FF)
}

fn prg8_offset(prg_len: usize, bank: usize, addr: u16) -> usize {
    (bank % (prg_len / 0x2000)) * 0x2000 + (addr as usize & 0x1FFF)
}

/// Taito X1-005 (mapper 80) and its nametable-banking wiring (mapper 207).
///
/// ```text
/// $7EF0/$7EF1  2KB CHR at $0000 / $0800 (mapper 207: bit 7 = CIRAM page
///              for $2000-$27FF / $2800-$2FFF)
/// $7EF2-$7EF5  1KB CHR at $1000 / $1400 / $1800 / $1C00
/// $7EF6/$7EF7  [.... ...M] mirroring: 0 horizontal, 1 vertical (mapper 80)
/// $7EF8/$7EF9  write $A3 to enable the internal RAM, anything else disables
/// $7EFA/$7EFB  8KB PRG at $8000
/// $7EFC/$7EFD  8KB PRG at $A000
/// $7EFE/$7EFF  8KB PRG at $C000      ($E000 fixed to the last bank)
/// $7F00-$7FFF  128 bytes of battery-backed RAM inside the chip (mirrored)
/// ```
/// Games: Minelvaton Saga, Kyonshiizu 2, Fudou Myouou Den (207),
/// Taito Grand Prix (207).
#[derive(Serialize, Deserialize)]
pub struct TaitoX1005 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    ram: Vec<u8>,
    ram_enabled: bool,
    chr_regs: [u8; 6],
    prg_banks: [u8; 3],
    mirroring: Mirroring,
    four_screen: bool,
    /// Mapper 207: nametables come from CHR register bit 7, not $7EF6.
    nt_banking: bool,
}

impl TaitoX1005 {
    pub fn new(mapper_id: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        TaitoX1005 {
            prg,
            chr,
            chr_is_ram,
            ram: vec![0; 0x80],
            ram_enabled: false,
            chr_regs: [0; 6],
            prg_banks: [0; 3],
            four_screen: mirroring == Mirroring::FourScreen,
            mirroring,
            nt_banking: mapper_id == 207,
        }
    }
}

impl Mapper for TaitoX1005 {
    crate::impl_mapper_savestate!(chr, ram);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        // The 128-byte RAM is inside the chip; only CHR RAM is sizeable.
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        let bank = match addr {
            0x8000..=0x9FFF => self.prg_banks[0] as usize,
            0xA000..=0xBFFF => self.prg_banks[1] as usize,
            0xC000..=0xDFFF => self.prg_banks[2] as usize,
            0xE000..=0xFFFF => self.prg.len() / 0x2000 - 1,
            _ => return 0,
        };
        self.prg[prg8_offset(self.prg.len(), bank, addr)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x7EF0..=0x7EF5 => self.chr_regs[(addr - 0x7EF0) as usize] = val,
            0x7EF6 | 0x7EF7 => {
                if !self.four_screen && !self.nt_banking {
                    self.mirroring = if val & 1 != 0 {
                        Mirroring::Vertical
                    } else {
                        Mirroring::Horizontal
                    };
                }
            }
            0x7EF8 | 0x7EF9 => self.ram_enabled = val == 0xA3,
            0x7EFA..=0x7EFF => self.prg_banks[((addr - 0x7EFA) / 2) as usize] = val,
            0x7F00..=0x7FFF if self.ram_enabled => self.ram[(addr & 0x7F) as usize] = val,
            _ => {}
        }
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        (addr >= 0x7F00 && self.ram_enabled).then(|| self.ram[(addr & 0x7F) as usize])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.ram)
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[x1_chr_offset(self.chr.len(), &self.chr_regs, false, addr)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram {
            let off = x1_chr_offset(self.chr.len(), &self.chr_regs, false, addr);
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn nt_target(&mut self, addr: u16) -> NtTarget {
        if self.nt_banking && !self.four_screen {
            // $2000-$27FF follows $7EF0 bit 7, $2800-$2FFF follows $7EF1.
            let reg = self.chr_regs[((addr >> 11) & 1) as usize];
            NtTarget::Ciram(((reg >> 7) as u16) << 10 | (addr & 0x3FF))
        } else {
            NtTarget::Ciram(super::mirror_nt(self.mirroring, addr))
        }
    }
}

/// Taito X1-017 (mapper 82).
///
/// ```text
/// $7EF0/$7EF1  2KB CHR at $0000 / $0800 (1KB bank number, bit 0 ignored)
/// $7EF2-$7EF5  1KB CHR at $1000-$1C00
/// $7EF6        [.... ..IM] M: 0 horizontal, 1 vertical; I swaps the 2KB
///              and 1KB halves of the pattern tables
/// $7EF7        $CA enables RAM $6000-$67FF
/// $7EF8        $69 enables RAM $6800-$6FFF
/// $7EF9        $84 enables RAM $7000-$73FF
/// $7EFA-$7EFC  8KB PRG at $8000 / $A000 / $C000 = value >> 2
/// ```
/// $E000 is fixed to the last bank. The 5KB of RAM is battery-backed.
/// Games: SD Keiji: Blader, Kyuukyoku Harikiri Stadium III.
#[derive(Serialize, Deserialize)]
pub struct TaitoX1017 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    ram: Vec<u8>,
    ram_enabled: [bool; 3],
    chr_regs: [u8; 6],
    chr_invert: bool,
    prg_banks: [u8; 3],
    mirroring: Mirroring,
    four_screen: bool,
}

/// Enable key and size of each X1-017 RAM chunk.
const X1017_RAM_KEYS: [u8; 3] = [0xCA, 0x69, 0x84];

impl TaitoX1017 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        TaitoX1017 {
            prg,
            chr,
            chr_is_ram,
            ram: vec![0; 0x1400],
            ram_enabled: [false; 3],
            chr_regs: [0; 6],
            chr_invert: false,
            prg_banks: [0; 3],
            four_screen: mirroring == Mirroring::FourScreen,
            mirroring,
        }
    }

    /// RAM index for $6000-$73FF if that chunk is enabled.
    fn ram_index(&self, addr: u16) -> Option<usize> {
        let chunk = match addr {
            0x6000..=0x67FF => 0,
            0x6800..=0x6FFF => 1,
            0x7000..=0x73FF => 2,
            _ => return None,
        };
        let i = addr as usize - 0x6000;
        (self.ram_enabled[chunk] && i < self.ram.len()).then_some(i)
    }
}

impl Mapper for TaitoX1017 {
    crate::impl_mapper_savestate!(chr, ram);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        let last = self.prg.len() / 0x2000 - 1;
        let bank = match addr {
            0x8000..=0x9FFF => self.prg_banks[0] as usize,
            0xA000..=0xBFFF => self.prg_banks[1] as usize,
            0xC000..=0xDFFF => self.prg_banks[2] as usize,
            0xE000..=0xFFFF => last,
            _ => return 0,
        };
        self.prg[prg8_offset(self.prg.len(), bank, addr)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x6000..=0x73FF => {
                if let Some(i) = self.ram_index(addr) {
                    self.ram[i] = val;
                }
            }
            0x7EF0..=0x7EF5 => self.chr_regs[(addr - 0x7EF0) as usize] = val,
            0x7EF6 => {
                if !self.four_screen {
                    self.mirroring = if val & 1 != 0 {
                        Mirroring::Vertical
                    } else {
                        Mirroring::Horizontal
                    };
                }
                self.chr_invert = val & 2 != 0;
            }
            0x7EF7..=0x7EF9 => {
                let chunk = (addr - 0x7EF7) as usize;
                self.ram_enabled[chunk] = val == X1017_RAM_KEYS[chunk];
            }
            // The PRG registers' bank number sits two bits up.
            0x7EFA..=0x7EFC => self.prg_banks[(addr - 0x7EFA) as usize] = val >> 2,
            _ => {}
        }
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        self.ram_index(addr).map(|i| self.ram[i])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.ram)
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[x1_chr_offset(self.chr.len(), &self.chr_regs, self.chr_invert, addr)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram {
            let off = x1_chr_offset(self.chr.len(), &self.chr_regs, self.chr_invert, addr);
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

    fn prg() -> Vec<u8> {
        (0..16 * 0x2000).map(|i| (i / 0x2000) as u8).collect()
    }

    fn chr() -> Vec<u8> {
        (0..128 * 0x400).map(|i| (i / 0x400) as u8).collect()
    }

    #[test]
    fn x1005_prg_chr_and_mirroring() {
        let mut m = TaitoX1005::new(80, prg(), chr(), Mirroring::Horizontal);
        m.cpu_write(0x7EFA, 3);
        m.cpu_write(0x7EFD, 5);
        m.cpu_write(0x7EFE, 9);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xA000), 5);
        assert_eq!(m.cpu_read(0xC000), 9);
        assert_eq!(m.cpu_read(0xE000), 15);
        m.cpu_write(0x7EF0, 0x15); // 2KB: bit 0 ignored -> banks $14/$15
        assert_eq!(m.ppu_read(0x0000), 0x14);
        assert_eq!(m.ppu_read(0x0400), 0x15);
        m.cpu_write(0x7EF5, 0x33);
        assert_eq!(m.ppu_read(0x1C00), 0x33);
        m.cpu_write(0x7EF6, 1);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn x1005_internal_ram_needs_key() {
        let mut m = TaitoX1005::new(80, prg(), chr(), Mirroring::Horizontal);
        m.cpu_write(0x7F00, 0x11);
        assert_eq!(m.prg_ram_read(0x7F00), None);
        m.cpu_write(0x7EF8, 0xA3);
        m.cpu_write(0x7F05, 0x22);
        assert_eq!(m.prg_ram_read(0x7F85), Some(0x22), "128 bytes, mirrored");
        assert_eq!(m.prg_ram_read(0x6000), None);
        m.cpu_write(0x7EF9, 0x00);
        assert_eq!(m.prg_ram_read(0x7F05), None);
        assert_eq!(m.prg_ram().unwrap().len(), 0x80);
    }

    #[test]
    fn mapper207_nametables_from_chr_bit7() {
        let mut m = TaitoX1005::new(207, prg(), chr(), Mirroring::Horizontal);
        m.cpu_write(0x7EF0, 0x80);
        m.cpu_write(0x7EF1, 0x00);
        assert_eq!(m.nt_target(0x2005), NtTarget::Ciram(0x405));
        assert_eq!(m.nt_target(0x2405), NtTarget::Ciram(0x405));
        assert_eq!(m.nt_target(0x2805), NtTarget::Ciram(0x005));
        // Bit 7 doesn't leak into the CHR bank.
        assert_eq!(m.ppu_read(0x0000), 0x00);
        // The mirroring register is ignored.
        m.cpu_write(0x7EF6, 1);
        assert_eq!(m.nt_target(0x2C00), NtTarget::Ciram(0x000));
    }

    #[test]
    fn x1017_prg_shift_ram_chunks_and_inversion() {
        let mut m = TaitoX1017::new(prg(), chr(), Mirroring::Horizontal);
        m.cpu_write(0x7EFA, 3 << 2);
        m.cpu_write(0x7EFC, 7 << 2);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xC000), 7);
        assert_eq!(m.cpu_read(0xE000), 15);
        // Only the keyed chunk opens.
        m.cpu_write(0x7EF8, 0x69);
        m.cpu_write(0x6000, 1);
        m.cpu_write(0x6800, 2);
        assert_eq!(m.prg_ram_read(0x6000), None);
        assert_eq!(m.prg_ram_read(0x6800), Some(2));
        m.cpu_write(0x7EF7, 0xCA);
        m.cpu_write(0x7EF9, 0x84);
        m.cpu_write(0x73FF, 3);
        assert_eq!(m.prg_ram_read(0x73FF), Some(3));
        assert_eq!(m.prg_ram_read(0x7400), None);
        // CHR inversion swaps the halves.
        m.cpu_write(0x7EF0, 0x20);
        m.cpu_write(0x7EF2, 0x40);
        assert_eq!(m.ppu_read(0x0000), 0x20);
        m.cpu_write(0x7EF6, 0x02);
        assert_eq!(m.ppu_read(0x1000), 0x20);
        assert_eq!(m.ppu_read(0x0000), 0x40);
    }
}
