//! Small discrete-logic boards built from a latch or two: one shared core,
//! with the register decode and bank layout picked by [`LatchKind`].
//!
//! - 13 CPROM: 32KB PRG, 16KB CHR RAM; $0000 fixed, $1000 a switchable 4KB
//!   bank ($8000-$FFFF, bits 0-1).
//! - 38 Bit Corp UNL-PCI556: $7000-$7FFF `[.... CCPP]` - 32KB PRG, 8KB CHR.
//! - 79 / 146 AVE NINA-03/06, Sachen 3015: `[.... PCCC]` at $4100-$5FFF
//!   (address & $E100 = $4100).
//! - 113 NINA-03/06 multicart variant: `[MCPP PCCC]` at the same address,
//!   M = mirroring (1 = vertical), C = CHR bit 3.
//! - 94 UN1ROM: `[...P PP..]` - UxROM with the bank in bits 2-4.
//! - 107 Magic Dragon: `[PPPP PPP.]` 32KB PRG, same byte selects 8KB CHR.
//! - 240: `[PPPP CCCC]` at $4020-$5FFF, 8KB PRG RAM.
//! - 241: BxROM-style 32KB PRG select with 8KB PRG RAM.
//! - 242 Waixing: address latch at $8000-$FFFF - A6-A3 select 32KB PRG,
//!   A1 mirroring (1 = horizontal); 8KB PRG RAM.
//!
//! Bus conflicts are not emulated (like the other discrete boards).

use super::Chr;

use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Index into a ROM/RAM of `len` bytes for byte `addr` of `unit`-sized bank
/// `bank`. The bank wraps modulo the bank count; an image smaller than one
/// bank mirrors across it. `len` must be non-zero and `unit` a power of two.
#[inline]
pub(crate) fn bank_index(len: usize, unit: usize, bank: usize, addr: usize) -> usize {
    let off = addr & (unit - 1);
    if len < unit {
        return off % len;
    }
    (bank % (len / unit)) * unit + off
}

/// CHR ROM as given, or `ram` bytes of CHR RAM when the header declares none
/// (the iNES CHR RAM convention). Returns the buffer and whether it is RAM.
pub(crate) fn chr_or_ram(chr: Vec<u8>, ram: usize) -> (Vec<u8>, bool) {
    if chr.is_empty() {
        (vec![0; ram], true)
    } else {
        (chr, false)
    }
}

/// NINA-03/06 register decode: $4100-$5FFF with address & $E100 = $4100.
fn nina_reg(addr: u16) -> bool {
    (0x4020..=0x5FFF).contains(&addr) && addr & 0xE100 == 0x4100
}

/// Which board a [`Latch`] models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LatchKind {
    Cprom,
    BitCorp38,
    Nina79,
    Nina113,
    Un1rom,
    MagicDragon,
    M240,
    M241,
    M242,
}

impl LatchKind {
    fn has_prg_ram(self) -> bool {
        matches!(self, LatchKind::M240 | LatchKind::M241 | LatchKind::M242)
    }
}

#[derive(Serialize, Deserialize)]
pub struct Latch {
    kind: LatchKind,
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Chr,
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Latch {
    pub fn new(kind: LatchKind, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let default_chr_ram = if kind == LatchKind::Cprom {
            0x4000
        } else {
            0x2000
        };
        let (chr, chr_is_ram) = chr_or_ram(chr, default_chr_ram);
        Latch {
            kind,
            prg,
            chr: chr.into(),
            chr_is_ram,
            prg_ram: if kind.has_prg_ram() {
                vec![0; 0x2000]
            } else {
                Vec::new()
            },
            prg_bank: 0,
            chr_bank: 0,
            mirroring,
        }
    }

    fn write_reg(&mut self, addr: u16, val: u8) {
        use LatchKind::*;
        let rom = addr >= 0x8000;
        match self.kind {
            Cprom if rom => self.chr_bank = val & 3,
            BitCorp38 if (0x7000..=0x7FFF).contains(&addr) => {
                self.prg_bank = val & 3;
                self.chr_bank = (val >> 2) & 3;
            }
            Nina79 if nina_reg(addr) => {
                self.prg_bank = (val >> 3) & 1;
                self.chr_bank = val & 7;
            }
            Nina113 if nina_reg(addr) => {
                self.prg_bank = (val >> 3) & 7;
                self.chr_bank = (val & 7) | ((val >> 3) & 8);
                self.mirroring = if val & 0x80 != 0 {
                    Mirroring::Vertical
                } else {
                    Mirroring::Horizontal
                };
            }
            Un1rom if rom => self.prg_bank = (val >> 2) & 7,
            MagicDragon if rom => {
                self.prg_bank = val >> 1;
                self.chr_bank = val;
            }
            M240 if (0x4020..=0x5FFF).contains(&addr) => {
                self.prg_bank = val >> 4;
                self.chr_bank = val & 0x0F;
            }
            M241 if rom => self.prg_bank = val,
            M242 if rom => {
                self.prg_bank = ((addr >> 3) & 0x0F) as u8;
                self.mirroring = if addr & 2 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            _ => {}
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let len = self.chr.len();
        let a = addr as usize;
        if self.kind == LatchKind::Cprom {
            let bank = if a < 0x1000 {
                0
            } else {
                self.chr_bank as usize
            };
            bank_index(len, 0x1000, bank, a)
        } else {
            bank_index(len, 0x2000, self.chr_bank as usize, a)
        }
    }
}

impl Mapper for Latch {
    crate::impl_mapper_savestate!(chr, prg_ram);

    fn set_ram_sizes(&mut self, prg_ram: usize, chr_ram: usize) {
        if prg_ram > 0 && self.kind.has_prg_ram() {
            self.prg_ram = vec![0; prg_ram];
        }
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram].into();
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let len = self.prg.len();
        let a = addr as usize;
        let i = match self.kind {
            LatchKind::Cprom => bank_index(len, 0x8000, 0, a),
            LatchKind::Un1rom => {
                let bank = if addr < 0xC000 {
                    self.prg_bank as usize
                } else {
                    len / 0x4000 - 1
                };
                bank_index(len, 0x4000, bank, a)
            }
            _ => bank_index(len, 0x8000, self.prg_bank as usize, a),
        };
        self.prg[i]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if (0x6000..=0x7FFF).contains(&addr) && !self.prg_ram.is_empty() {
            let n = self.prg_ram.len();
            self.prg_ram[(addr as usize - 0x6000) % n] = val;
            return;
        }
        self.write_reg(addr, val);
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        let n = self.prg_ram.len();
        (n > 0).then(|| self.prg_ram[(addr as usize - 0x6000) % n])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        (!self.prg_ram.is_empty()).then_some(self.prg_ram.as_slice())
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        (!self.prg_ram.is_empty()).then_some(self.prg_ram.as_mut_slice())
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
pub(crate) mod tests {
    use super::*;

    /// `n` banks of `unit` bytes, each byte = its bank index.
    pub(crate) fn banks(n: usize, unit: usize) -> Vec<u8> {
        (0..n * unit).map(|i| (i / unit) as u8).collect()
    }

    fn latch(kind: LatchKind, prg_32k: usize, chr_8k: usize) -> Latch {
        Latch::new(
            kind,
            banks(prg_32k * 2, 0x4000),
            banks(chr_8k, 0x2000),
            Mirroring::Vertical,
        )
    }

    #[test]
    fn bank_index_wraps_and_mirrors() {
        assert_eq!(bank_index(0x8000, 0x4000, 3, 0x8123), 0x4123);
        assert_eq!(bank_index(0x4000, 0x8000, 5, 0xC123), 0x0123);
    }

    #[test]
    fn cprom_switches_upper_4k_of_chr_ram() {
        let mut m = Latch::new(
            LatchKind::Cprom,
            vec![0; 0x8000],
            vec![],
            Mirroring::Vertical,
        );
        m.ppu_write(0x1000, 0xAA); // bank 0 at $1000 = CHR $0000
        assert_eq!(m.ppu_read(0x0000), 0xAA);
        m.cpu_write(0x8000, 2);
        m.ppu_write(0x1000, 0x55);
        assert_eq!(m.ppu_read(0x0000), 0xAA, "$0000 stays fixed");
        assert_eq!(m.ppu_read(0x1000), 0x55);
        m.cpu_write(0x8000, 0);
        assert_eq!(m.ppu_read(0x1000), 0xAA);
    }

    #[test]
    fn bitcorp38_register_at_7000() {
        let mut m = latch(LatchKind::BitCorp38, 4, 4);
        m.cpu_write(0x8000, 0x0F); // not a register
        assert_eq!(m.cpu_read(0x8000), 0);
        m.cpu_write(0x7000, 0b1110);
        assert_eq!(m.cpu_read(0x8000), 4, "32KB bank 2 = 16KB bank 4");
        assert_eq!(m.ppu_read(0), 3);
    }

    #[test]
    fn nina79_decodes_e100_mask() {
        let mut m = latch(LatchKind::Nina79, 2, 8);
        m.cpu_write(0x4000 | 0x0200, 0x0F); // A8 clear: ignored
        assert_eq!(m.ppu_read(0), 0);
        m.cpu_write(0x5D00, 0x0D); // $5D00 & $E100 = $4100
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.ppu_read(0), 5);
    }

    #[test]
    fn nina113_bank_bits_and_mirroring() {
        let mut m = latch(LatchKind::Nina113, 8, 16);
        m.cpu_write(0x4100, 0b1110_1011); // M=1, C3=1, PPP=5, CCC=3
        assert_eq!(m.cpu_read(0x8000), 10);
        assert_eq!(m.ppu_read(0), 11);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0x4100, 0);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn un1rom_bank_in_bits_2_to_4() {
        let mut m = Latch::new(
            LatchKind::Un1rom,
            banks(8, 0x4000),
            vec![],
            Mirroring::Vertical,
        );
        m.cpu_write(0x8000, 3 << 2);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xC000), 7, "last bank fixed");
    }

    #[test]
    fn magic_dragon_and_240_241_242() {
        let mut m = latch(LatchKind::MagicDragon, 4, 8);
        m.cpu_write(0x8000, 0x07);
        assert_eq!(m.cpu_read(0x8000), 6);
        assert_eq!(m.ppu_read(0), 7);

        let mut m = latch(LatchKind::M240, 4, 8);
        m.cpu_write(0x4800, 0x25);
        assert_eq!(m.cpu_read(0x8000), 4);
        assert_eq!(m.ppu_read(0), 5);
        m.cpu_write(0x6001, 0x77);
        assert_eq!(m.prg_ram_read(0x6001), Some(0x77));

        let mut m = latch(LatchKind::M241, 4, 1);
        m.cpu_write(0x8000, 3);
        assert_eq!(m.cpu_read(0xFFFF), 7);
        assert!(m.prg_ram().is_some());

        let mut m = latch(LatchKind::M242, 4, 1);
        m.cpu_write(0x8000 | (2 << 3) | 2, 0);
        assert_eq!(m.cpu_read(0x8000), 4);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn savestate_roundtrip() {
        let mut m = latch(LatchKind::M240, 4, 8);
        m.cpu_write(0x4800, 0x13);
        m.cpu_write(0x6000, 0x42);
        let s = m.save_state();
        let mut n = latch(LatchKind::M240, 4, 8);
        n.load_state(&s).unwrap();
        assert_eq!(n.cpu_read(0x8000), 2);
        assert_eq!(n.prg_ram_read(0x6000), Some(0x42));
    }
}
