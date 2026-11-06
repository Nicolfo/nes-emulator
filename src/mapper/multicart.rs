//! Discrete-logic multicart boards (pirate and licensed compilations).
//!
//! - 15 K-1029/K-1030P "100-in-1 Contra Function 16"
//! - 41 Caltron 6-in-1
//! - 46 Color Dreams Rumble Station 15-in-1
//! - 228 Active Enterprises Action 52 / Cheetahmen II
//! - 232 Camerica BF9096 Quattro (submapper 1: Aladdin Deck Enhancer)

use super::discrete::{bank_index, chr_or_ram};
use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Implements CHR read/write and `set_ram_sizes` for a board whose CHR is a
/// single bank of `unit` bytes chosen by `self.chr_bank()`.
macro_rules! chr_impl {
    ($unit:expr) => {
        fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
            if chr_ram > 0 && self.chr_is_ram {
                self.chr = vec![0; chr_ram];
            }
        }

        fn ppu_read(&mut self, addr: u16) -> u8 {
            self.chr[bank_index(self.chr.len(), $unit, self.chr_bank(), addr as usize)]
        }

        fn ppu_write(&mut self, addr: u16, val: u8) {
            if self.chr_is_ram && self.chr_writable() {
                let i = bank_index(self.chr.len(), $unit, self.chr_bank(), addr as usize);
                self.chr[i] = val;
            }
        }
    };
}

// ---------------------------------------------------------------------------

/// Mapper 15 (K-1029 / K-1030P). Writes to $8000-$FFFF latch the data byte
/// `[SMPP PPPP]` and the PRG mode from address bits A1-A0:
/// - 0 NROM-256: 16KB banks P at $8000 and P|1 at $C000
/// - 1 UNROM: P at $8000, P|7 at $C000
/// - 2 NROM-64: 8KB bank (P*2 + S) in all four slots
/// - 3 NROM-128: P at both $8000 and $C000
///
/// M = mirroring (1 = horizontal). 8KB CHR RAM, write-protected in modes 0
/// and 3.
#[derive(Serialize, Deserialize)]
pub struct K1029 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mode: u8,
    reg: u8,
}

impl K1029 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x2000);
        K1029 {
            prg,
            chr,
            chr_is_ram,
            mode: 0,
            reg: 0,
        }
    }

    fn chr_bank(&self) -> usize {
        0
    }

    fn chr_writable(&self) -> bool {
        matches!(self.mode, 1 | 2)
    }
}

impl Mapper for K1029 {
    crate::impl_mapper_savestate!(chr);
    chr_impl!(0x2000);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let p = (self.reg & 0x3F) as usize;
        let hi = addr >= 0xC000;
        let len = self.prg.len();
        let a = addr as usize;
        let i = match self.mode {
            0 => bank_index(len, 0x4000, if hi { p | 1 } else { p }, a),
            1 => bank_index(len, 0x4000, if hi { p | 7 } else { p }, a),
            2 => bank_index(len, 0x2000, (p << 1) | (self.reg >> 7) as usize, a),
            _ => bank_index(len, 0x4000, p, a),
        };
        self.prg[i]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            self.mode = (addr & 3) as u8;
            self.reg = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        if self.reg & 0x40 != 0 {
            Mirroring::Horizontal
        } else {
            Mirroring::Vertical
        }
    }
}

// ---------------------------------------------------------------------------

/// Mapper 41 (Caltron 6-in-1). The outer register is the *address* of a
/// write to $6000-$67FF: `A~[..MH HEPP]` - E+PP select a 32KB PRG bank, HH
/// the high CHR bits, M mirroring (1 = horizontal). E also unlocks the inner
/// register at $8000-$FFFF, whose data bits 0-1 are the low CHR bits.
#[derive(Serialize, Deserialize)]
pub struct Caltron {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    outer: u8,
    inner: u8,
}

impl Caltron {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x2000);
        Caltron {
            prg,
            chr,
            chr_is_ram,
            outer: 0,
            inner: 0,
        }
    }

    fn chr_bank(&self) -> usize {
        ((((self.outer >> 3) & 3) << 2) | self.inner) as usize
    }

    fn chr_writable(&self) -> bool {
        true
    }
}

impl Mapper for Caltron {
    crate::impl_mapper_savestate!(chr);
    chr_impl!(0x2000);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            let bank = (self.outer & 7) as usize;
            self.prg[bank_index(self.prg.len(), 0x8000, bank, addr as usize)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x6000..=0x67FF => self.outer = (addr & 0x3F) as u8,
            0x8000..=0xFFFF if self.outer & 4 != 0 => self.inner = val & 3,
            _ => {}
        }
    }

    fn mirroring(&self) -> Mirroring {
        if self.outer & 0x20 != 0 {
            Mirroring::Horizontal
        } else {
            Mirroring::Vertical
        }
    }
}

// ---------------------------------------------------------------------------

/// Mapper 46 (Rumble Station 15-in-1): Color Dreams boards behind an outer
/// register. $6000-$7FFF `[CCCC PPPP]` holds the high bits, $8000-$FFFF
/// `[.CCC ...P]` the low bits: 32KB PRG = outer P << 1 | inner P, 8KB CHR =
/// outer C << 3 | inner C. Mirroring is hardwired.
#[derive(Serialize, Deserialize)]
pub struct RumbleStation {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    outer: u8,
    inner: u8,
    mirroring: Mirroring,
}

impl RumbleStation {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x2000);
        RumbleStation {
            prg,
            chr,
            chr_is_ram,
            outer: 0,
            inner: 0,
            mirroring,
        }
    }

    fn chr_bank(&self) -> usize {
        (((self.outer >> 4) as usize) << 3) | ((self.inner >> 4) & 7) as usize
    }

    fn chr_writable(&self) -> bool {
        true
    }
}

impl Mapper for RumbleStation {
    crate::impl_mapper_savestate!(chr);
    chr_impl!(0x2000);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            let bank = (((self.outer & 0x0F) as usize) << 1) | (self.inner & 1) as usize;
            self.prg[bank_index(self.prg.len(), 0x8000, bank, addr as usize)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x6000..=0x7FFF => self.outer = val,
            0x8000..=0xFFFF => self.inner = val,
            _ => {}
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }
}

// ---------------------------------------------------------------------------

/// Mapper 228 (Action 52 / Cheetahmen II). A write to $8000-$FFFF latches
/// its address `A~[..MH HPPP PPO. CCCC]` and data `D~[.... ..cc]`:
/// - HH: PRG chip (512KB each; chip 3 is the third physical chip, chip 2 is
///   absent), PPPPP: 16KB bank within the chip
/// - O: 1 = 16KB bank mirrored at $8000/$C000, 0 = 32KB (bank & !1, bank | 1)
/// - M: mirroring (1 = horizontal)
/// - CHR 8KB bank = CCCC << 2 | cc
///
/// $4020-$5FFF holds four mirrored 4-bit RAM cells.
#[derive(Serialize, Deserialize)]
pub struct Action52 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    addr_latch: u16,
    data_latch: u8,
    nibbles: [u8; 4],
}

impl Action52 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x2000);
        Action52 {
            prg,
            chr,
            chr_is_ram,
            addr_latch: 0,
            data_latch: 0,
            nibbles: [0; 4],
        }
    }

    fn chr_bank(&self) -> usize {
        (((self.addr_latch & 0x0F) as usize) << 2) | (self.data_latch & 3) as usize
    }

    fn chr_writable(&self) -> bool {
        true
    }
}

impl Mapper for Action52 {
    crate::impl_mapper_savestate!(chr);
    chr_impl!(0x2000);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let a = self.addr_latch as usize;
        let chip = match (a >> 11) & 3 {
            3 => 2,
            c => c,
        };
        let bank = chip * 32 + ((a >> 6) & 0x1F);
        let bank = if a & 0x20 != 0 {
            bank
        } else if addr < 0xC000 {
            bank & !1
        } else {
            bank | 1
        };
        self.prg[bank_index(self.prg.len(), 0x4000, bank, addr as usize)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x4020..=0x5FFF => self.nibbles[(addr & 3) as usize] = val & 0x0F,
            0x8000..=0xFFFF => {
                self.addr_latch = addr & 0x3FFF;
                self.data_latch = val;
            }
            _ => {}
        }
    }

    fn cpu_reg_read(&mut self, addr: u16) -> Option<u8> {
        Some(self.nibbles[(addr & 3) as usize])
    }

    fn mirroring(&self) -> Mirroring {
        if self.addr_latch & 0x2000 != 0 {
            Mirroring::Horizontal
        } else {
            Mirroring::Vertical
        }
    }
}

// ---------------------------------------------------------------------------

/// Mapper 232 (Camerica BF9096, Quattro series). $8000-$BFFF `[...B B...]`
/// selects a 64KB block, $C000-$FFFF `[.... ..PP]` a 16KB bank in it; the
/// block's last bank is fixed at $C000. Submapper 1 (Aladdin Deck Enhancer)
/// wires the two block bits in reverse order. CHR RAM.
#[derive(Serialize, Deserialize)]
pub struct Quattro {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    aladdin: bool,
    block: u8,
    inner: u8,
    mirroring: Mirroring,
}

impl Quattro {
    pub fn new(submapper: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x2000);
        Quattro {
            prg,
            chr,
            chr_is_ram,
            aladdin: submapper == 1,
            block: 0,
            inner: 0,
            mirroring,
        }
    }

    fn chr_bank(&self) -> usize {
        0
    }

    fn chr_writable(&self) -> bool {
        true
    }
}

impl Mapper for Quattro {
    crate::impl_mapper_savestate!(chr);
    chr_impl!(0x2000);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let inner = if addr < 0xC000 { self.inner } else { 3 };
        let bank = (self.block as usize) << 2 | inner as usize;
        self.prg[bank_index(self.prg.len(), 0x4000, bank, addr as usize)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x8000..=0xBFFF => {
                let b = (val >> 3) & 3;
                self.block = if self.aladdin {
                    ((b & 1) << 1) | (b >> 1)
                } else {
                    b
                };
            }
            0xC000..=0xFFFF => self.inner = val & 3,
            _ => {}
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::discrete::tests::banks;

    #[test]
    fn k1029_prg_modes_and_chr_protect() {
        let mut m = K1029::new(banks(64, 0x4000), vec![]);
        m.cpu_write(0x8000, 0x04); // mode 0, P=4
        assert_eq!((m.cpu_read(0x8000), m.cpu_read(0xC000)), (4, 5));
        m.ppu_write(0, 0x11);
        assert_eq!(m.ppu_read(0), 0, "CHR RAM protected in mode 0");
        m.cpu_write(0x8001, 0x48); // mode 1, P=8, horizontal
        assert_eq!((m.cpu_read(0x8000), m.cpu_read(0xC000)), (8, 15));
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.ppu_write(0, 0x11);
        assert_eq!(m.ppu_read(0), 0x11);
        m.cpu_write(0x8002, 0x83); // mode 2, 8KB bank 7 = upper half of 16KB bank 3
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xE000), 3);
        m.cpu_write(0x8003, 0x09); // mode 3
        assert_eq!((m.cpu_read(0x8000), m.cpu_read(0xC000)), (9, 9));
    }

    #[test]
    fn caltron_outer_unlocks_inner() {
        let mut m = Caltron::new(banks(16, 0x4000), banks(16, 0x2000));
        m.cpu_write(0x8000, 3); // locked: E clear
        assert_eq!(m.ppu_read(0), 0);
        m.cpu_write(0x6000 | 0x20 | 0x08 | 0x05, 0); // M, H=1, E=1, PP=1
        assert_eq!(m.cpu_read(0x8000), 10, "32KB bank 5");
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0x8000, 3);
        assert_eq!(m.ppu_read(0), 7);
    }

    #[test]
    fn rumble_station_combines_outer_and_inner() {
        let mut m = RumbleStation::new(banks(64, 0x4000), banks(64, 0x2000), Mirroring::Vertical);
        m.cpu_write(0x6000, 0x23);
        m.cpu_write(0x8000, 0x51);
        assert_eq!(m.cpu_read(0x8000), 14, "32KB bank 7");
        assert_eq!(m.ppu_read(0), 21);
    }

    #[test]
    fn action52_banking_and_nibble_ram() {
        let mut m = Action52::new(banks(96, 0x4000), banks(64, 0x2000));
        // chip 3 (-> 2), bank 5, 32KB mode, CHR high 1, data low 2, horizontal
        let addr = 0x8000 | 0x2000 | (3 << 11) | (5 << 6) | 0x01;
        m.cpu_write(addr, 0x02);
        assert_eq!(m.cpu_read(0x8000), 68);
        assert_eq!(m.cpu_read(0xC000), 69);
        assert_eq!(m.ppu_read(0), 6);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0x8000 | (5 << 6) | 0x20, 0); // 16KB mode
        assert_eq!((m.cpu_read(0x8000), m.cpu_read(0xC000)), (5, 5));
        m.cpu_write(0x5FF2, 0xAB);
        assert_eq!(m.cpu_reg_read(0x4022), Some(0x0B));
    }

    #[test]
    fn quattro_blocks() {
        let mut m = Quattro::new(0, banks(16, 0x4000), vec![], Mirroring::Vertical);
        m.cpu_write(0x8000, 0x10); // block 2
        m.cpu_write(0xC000, 1);
        assert_eq!((m.cpu_read(0x8000), m.cpu_read(0xC000)), (9, 11));
        let mut a = Quattro::new(1, banks(16, 0x4000), vec![], Mirroring::Vertical);
        a.cpu_write(0x8000, 0x10); // reversed bits: block 1
        assert_eq!(a.cpu_read(0xC000), 7);
    }

    #[test]
    fn savestate_roundtrip() {
        let mut m = Action52::new(banks(96, 0x4000), banks(64, 0x2000));
        m.cpu_write(0x8000 | (7 << 6) | 0x20, 0);
        m.cpu_write(0x4021, 9);
        let s = m.save_state();
        let mut n = Action52::new(banks(96, 0x4000), banks(64, 0x2000));
        n.load_state(&s).unwrap();
        assert_eq!(n.cpu_read(0x8000), 7);
        assert_eq!(n.cpu_reg_read(0x4021), Some(9));
    }
}
