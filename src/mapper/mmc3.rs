use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Boards built around a stock MMC3 that add outer banking or mix CHR RAM
/// into the CHR ROM bank space.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mmc3Board {
    /// Plain TxROM (mapper 4).
    #[default]
    Standard,
    /// Mapper 37, PAL-ZZ (Super Mario Bros. + Tetris + Nintendo World Cup):
    /// a 3-bit block register at $6000-$7FFF picks 64/128KB PRG blocks and a
    /// 128KB CHR half.
    PalZz,
    /// Mapper 47, NES-QJ (Super Spike V'Ball + Nintendo World Cup): bit 0 of
    /// $6000-$7FFF selects the 128KB PRG / 128KB CHR half.
    Qj,
    /// Mapper 119, TQROM: CHR bank bit 6 selects 8KB of CHR RAM instead of
    /// CHR ROM (High Speed, Pin-Bot).
    Tqrom,
    /// Mapper 74: 1KB CHR banks 8 and 9 are 2KB of CHR RAM.
    ChrRam74,
    /// Mapper 191: CHR banks with bit 7 set select 2KB of CHR RAM.
    ChrRam191,
    /// Mapper 192: CHR banks 8-11 are 4KB of CHR RAM.
    ChrRam192,
    /// Mapper 194: CHR banks 0 and 1 are 2KB of CHR RAM.
    ChrRam194,
    /// Mapper 195: CHR banks 0-3 are 4KB of CHR RAM.
    ChrRam195,
}

impl Mmc3Board {
    /// CHR RAM the board carries alongside CHR ROM (bytes).
    fn extra_chr_ram(self) -> usize {
        match self {
            Mmc3Board::Tqrom => 0x2000,
            Mmc3Board::ChrRam74 | Mmc3Board::ChrRam191 | Mmc3Board::ChrRam194 => 0x800,
            Mmc3Board::ChrRam192 | Mmc3Board::ChrRam195 => 0x1000,
            _ => 0,
        }
    }
}

/// Where a pattern fetch lands.
enum ChrLoc {
    Rom(usize),
    Ram(usize),
}

/// MMC3 (mapper 4): 8KB PRG banking, 1KB CHR banking, switchable mirroring,
/// and a scanline IRQ counter clocked by rising edges on PPU A12.
/// [`Mmc3Board`] variants add the multicart outer banks and CHR-RAM mixing
/// of mappers 37, 47, 74, 119, 191, 192, 194 and 195.
#[derive(Serialize, Deserialize)]
pub struct Mmc3 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    #[serde(default)]
    board: Mmc3Board,
    /// Multicart block register (mappers 37/47), written at $6000-$7FFF.
    #[serde(default)]
    outer: u8,
    /// Board CHR RAM mixed into the CHR ROM bank space (see [`Mmc3Board`]).
    #[serde(default)]
    chr_ram: Vec<u8>,
    prg_ram: Vec<u8>,
    mirroring: Mirroring,
    // $8000: bits 0-2 select which bank register $8001 writes; bit 6 PRG
    // mode; bit 7 CHR A12 inversion.
    bank_select: u8,
    bank_regs: [u8; 8],
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_line: bool,
    last_a12: bool,
    // $A001: bit 7 enables PRG RAM, bit 6 write-protects it. Power-on is
    // undefined on hardware; default to enabled + writable so games that
    // never touch $A001 keep working.
    ram_protect: u8,
}

impl Mmc3 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        Self::with_board(Mmc3Board::Standard, prg, chr, mirroring)
    }

    pub fn with_board(board: Mmc3Board, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        // A CHR-RAM-only dump of a mixing board has nothing to mix with: the
        // whole space is already RAM.
        let chr_ram = if chr_is_ram {
            Vec::new()
        } else {
            vec![0; board.extra_chr_ram()]
        };
        Mmc3 {
            prg,
            chr,
            chr_is_ram,
            board,
            outer: 0,
            chr_ram,
            prg_ram: vec![0; 0x2000],
            mirroring,
            bank_select: 0,
            bank_regs: [0; 8],
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_line: false,
            last_a12: false,
            ram_protect: 0x80,
        }
    }

    /// Map a CPU address ($8000-$FFFF) to a PRG ROM offset (8KB banks).
    fn prg_offset(&self, addr: u16) -> usize {
        if matches!(self.board, Mmc3Board::PalZz | Mmc3Board::Qj) {
            return self.multicart_prg_offset(addr);
        }
        let banks = self.prg.len() / 0x2000;
        let last = banks - 1;
        let mode = self.bank_select & 0x40 != 0;
        let bank = match (addr >> 13) & 3 {
            0 => {
                if mode {
                    last - 1
                } else {
                    self.bank_regs[6] as usize % banks
                }
            }
            1 => self.bank_regs[7] as usize % banks,
            2 => {
                if mode {
                    self.bank_regs[6] as usize % banks
                } else {
                    last - 1
                }
            }
            _ => last,
        };
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    /// PRG mapping for the multicart boards: the MMC3 picks an 8KB page
    /// (fixed pages are $3E/$3F) and the block register supplies the high
    /// bits.
    fn multicart_prg_offset(&self, addr: u16) -> usize {
        let mode = self.bank_select & 0x40 != 0;
        let page = match (addr >> 13) & 3 {
            0 if mode => 0x3E,
            0 => self.bank_regs[6] as usize,
            1 => self.bank_regs[7] as usize,
            2 if mode => self.bank_regs[6] as usize,
            2 => 0x3E,
            _ => 0x3F,
        };
        let page = match self.board {
            // Blocks 0-2: first 64KB; 3: second 64KB; 4-6: 128KB at
            // $20000; 7: last 64KB.
            Mmc3Board::PalZz => match self.outer & 7 {
                0..=2 => page & 7,
                3 => (page & 7) | 0x08,
                7 => (page & 7) | 0x18,
                _ => (page & 0x0F) | 0x10,
            },
            _ => (page & 0x0F) | ((self.outer as usize & 1) << 4),
        };
        (page % (self.prg.len() / 0x2000)) * 0x2000 + (addr as usize & 0x1FFF)
    }

    /// Map a PPU address ($0000-$1FFF) to CHR ROM or board CHR RAM
    /// (1KB banks).
    fn chr_loc(&self, addr: u16) -> ChrLoc {
        // Bit 7 swaps the 2KB and 1KB halves of the pattern space.
        let a = if self.bank_select & 0x80 != 0 {
            addr ^ 0x1000
        } else {
            addr
        };
        let bank = match a >> 10 {
            0 => self.bank_regs[0] as usize & !1,
            1 => self.bank_regs[0] as usize | 1,
            2 => self.bank_regs[1] as usize & !1,
            3 => self.bank_regs[1] as usize | 1,
            k => self.bank_regs[k as usize - 2] as usize,
        };
        let low = addr as usize & 0x3FF;
        if !self.chr_ram.is_empty() {
            let ram_bank = match self.board {
                Mmc3Board::Tqrom => (bank & 0x40 != 0).then_some(bank & 7),
                Mmc3Board::ChrRam74 => matches!(bank, 8 | 9).then_some(bank & 1),
                Mmc3Board::ChrRam191 => (bank & 0x80 != 0).then_some(bank & 1),
                Mmc3Board::ChrRam192 => (8..=11).contains(&bank).then_some(bank & 3),
                Mmc3Board::ChrRam194 => (bank < 2).then_some(bank),
                Mmc3Board::ChrRam195 => (bank < 4).then_some(bank),
                _ => None,
            };
            if let Some(r) = ram_bank {
                let banks = self.chr_ram.len() / 0x400;
                return ChrLoc::Ram((r % banks) * 0x400 + low);
            }
        }
        let bank = match self.board {
            Mmc3Board::PalZz => (bank & 0x7F) | ((self.outer as usize & 4) << 5),
            Mmc3Board::Qj => (bank & 0x7F) | ((self.outer as usize & 1) << 7),
            Mmc3Board::Tqrom => bank & 0x3F,
            _ => bank,
        };
        ChrLoc::Rom((bank % (self.chr.len() / 0x400)) * 0x400 + low)
    }

    /// Whether a $6000-$7FFF write lands (WRAM enabled and not protected) -
    /// the same gate the multicart block register sits behind.
    fn wram_writable(&self) -> bool {
        self.ram_protect & 0xC0 == 0x80
    }

    /// Clock the IRQ counter on each A12 rising edge seen on the PPU bus.
    fn watch_a12(&mut self, addr: u16) {
        let a12 = addr & 0x1000 != 0;
        if a12 && !self.last_a12 {
            if self.irq_counter == 0 || self.irq_reload {
                self.irq_counter = self.irq_latch;
                self.irq_reload = false;
            } else {
                self.irq_counter -= 1;
            }
            if self.irq_counter == 0 && self.irq_enabled {
                self.irq_line = true;
            }
        }
        self.last_a12 = a12;
    }
}

impl Mapper for Mmc3 {
    crate::impl_mapper_savestate!(chr, prg_ram, chr_ram);

    fn set_ram_sizes(&mut self, prg_ram: usize, chr_ram: usize) {
        if prg_ram > 0 {
            self.prg_ram = vec![0; prg_ram];
        }
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
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
        match addr {
            0x6000..=0x7FFF => {
                if self.wram_writable() {
                    match self.board {
                        // The multicarts decode $6000-$7FFF as their block
                        // register instead of work RAM.
                        Mmc3Board::PalZz | Mmc3Board::Qj => self.outer = val,
                        _ => self.prg_ram[(addr & 0x1FFF) as usize] = val,
                    }
                }
            }
            0x8000..=0x9FFF => {
                if addr & 1 == 0 {
                    self.bank_select = val;
                } else {
                    self.bank_regs[(self.bank_select & 7) as usize] = val;
                }
            }
            0xA000..=0xBFFF => {
                if addr & 1 == 0 {
                    // A four-screen board ignores the mirroring register.
                    if self.mirroring != Mirroring::FourScreen {
                        self.mirroring = if val & 1 != 0 {
                            Mirroring::Horizontal
                        } else {
                            Mirroring::Vertical
                        };
                    }
                } else {
                    self.ram_protect = val;
                }
            }
            0xC000..=0xDFFF => {
                if addr & 1 == 0 {
                    self.irq_latch = val;
                } else {
                    self.irq_counter = 0;
                    self.irq_reload = true;
                }
            }
            0xE000..=0xFFFF => {
                if addr & 1 == 0 {
                    self.irq_enabled = false;
                    self.irq_line = false;
                } else {
                    self.irq_enabled = true;
                }
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.watch_a12(addr);
        match self.chr_loc(addr) {
            ChrLoc::Rom(off) => self.chr[off],
            ChrLoc::Ram(off) => self.chr_ram[off],
        }
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        self.watch_a12(addr);
        match self.chr_loc(addr) {
            ChrLoc::Ram(off) => self.chr_ram[off] = val,
            ChrLoc::Rom(off) if self.chr_is_ram => self.chr[off] = val,
            ChrLoc::Rom(_) => {}
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        // The multicarts have no work RAM behind their block register.
        if self.ram_protect & 0x80 == 0 || matches!(self.board, Mmc3Board::PalZz | Mmc3Board::Qj) {
            return None; // open bus
        }
        Some(self.prg_ram[(addr & 0x1FFF) as usize])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
    }

    fn irq(&self) -> bool {
        self.irq_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mmc3() -> Mmc3 {
        // 4 PRG banks (32KB), 8 CHR banks (8KB); each byte = its bank index.
        let prg: Vec<u8> = (0..4 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Mmc3::new(prg, chr, Mirroring::Horizontal)
    }

    #[test]
    fn prg_fixed_banks() {
        let mut m = mmc3();
        // Mode 0: $C000 = second-last, $E000 = last.
        assert_eq!(m.cpu_read(0xC000), 2);
        assert_eq!(m.cpu_read(0xE000), 3);
        // Mode 1: $8000 = second-last instead.
        m.cpu_write(0x8000, 0x40);
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xE000), 3);
    }

    #[test]
    fn prg_switchable_banks() {
        let mut m = mmc3();
        m.cpu_write(0x8000, 6); // select R6
        m.cpu_write(0x8001, 1);
        m.cpu_write(0x8000, 7); // select R7
        m.cpu_write(0x8001, 0);
        assert_eq!(m.cpu_read(0x8000), 1);
        assert_eq!(m.cpu_read(0xA000), 0);
    }

    #[test]
    fn chr_banking_and_inversion() {
        let mut m = mmc3();
        m.cpu_write(0x8000, 0); // R0: 2KB at $0000 (low bit ignored)
        m.cpu_write(0x8001, 5);
        m.cpu_write(0x8000, 2); // R2: 1KB at $1000
        m.cpu_write(0x8001, 7);
        assert_eq!(m.ppu_read(0x0000), 4);
        assert_eq!(m.ppu_read(0x0400), 5);
        assert_eq!(m.ppu_read(0x1000), 7);
        // Bit 7 swaps the halves.
        m.cpu_write(0x8000, 0x80);
        assert_eq!(m.ppu_read(0x1000), 4);
        assert_eq!(m.ppu_read(0x0000), 7);
    }

    #[test]
    fn mirroring_control() {
        let mut m = mmc3();
        m.cpu_write(0xA000, 0);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0xA000, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn four_screen_ignores_mirroring_register() {
        let prg: Vec<u8> = (0..4 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x400).map(|i| (i / 0x400) as u8).collect();
        let mut m = Mmc3::new(prg, chr, Mirroring::FourScreen);
        m.cpu_write(0xA000, 0);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
        m.cpu_write(0xA000, 1);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }

    #[test]
    fn irq_counts_a12_rises() {
        let mut m = mmc3();
        m.cpu_write(0xC000, 3); // latch
        m.cpu_write(0xC001, 0); // reload on next clock
        m.cpu_write(0xE001, 0); // enable
        // Each low->high A12 transition is one clock.
        for i in 0..3 {
            m.ppu_read(0x0000); // A12 low
            m.ppu_read(0x1000); // A12 rise: reload to 3, then 2, 1
            assert!(!m.irq(), "IRQ too early at clock {i}");
        }
        m.ppu_read(0x0000);
        m.ppu_read(0x1000); // counter hits 0
        assert!(m.irq());
        // $E000 acknowledges and disables.
        m.cpu_write(0xE000, 0);
        assert!(!m.irq());
    }

    #[test]
    fn prg_ram_protect() {
        let mut m = mmc3();
        m.cpu_write(0x6000, 0xAA);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
        // Write-protected: writes ignored, reads still work.
        m.cpu_write(0xA001, 0xC0);
        m.cpu_write(0x6000, 0xBB);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
        // Disabled: open bus.
        m.cpu_write(0xA001, 0x00);
        assert_eq!(m.prg_ram_read(0x6000), None);
        // Re-enabled writable.
        m.cpu_write(0xA001, 0x80);
        m.cpu_write(0x6000, 0xBB);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xBB));
    }

    /// 512KB PRG / 256KB CHR image; each byte = its 8KB PRG page or 1KB
    /// CHR page index (CHR wraps at 256).
    fn board(b: Mmc3Board, prg_pages: usize, chr_pages: usize) -> Mmc3 {
        let prg: Vec<u8> = (0..prg_pages * 0x2000)
            .map(|i| (i / 0x2000) as u8)
            .collect();
        let chr: Vec<u8> = (0..chr_pages * 0x400).map(|i| (i / 0x400) as u8).collect();
        Mmc3::with_board(b, prg, chr, Mirroring::Vertical)
    }

    fn set_reg(m: &mut Mmc3, r: u8, v: u8) {
        m.cpu_write(0x8000, r);
        m.cpu_write(0x8001, v);
    }

    #[test]
    fn pal_zz_blocks() {
        // 256KB PRG = 32 pages, 256KB CHR.
        let mut m = board(Mmc3Board::PalZz, 32, 256);
        set_reg(&mut m, 6, 0x0B);
        assert_eq!(m.cpu_read(0x8000), 3, "block 0: 64KB, page & 7");
        assert_eq!(m.cpu_read(0xE000), 7, "fixed last page of the block");
        m.cpu_write(0x6000, 3);
        assert_eq!(m.cpu_read(0x8000), 0x0B);
        assert_eq!(m.cpu_read(0xE000), 0x0F);
        m.cpu_write(0x6000, 4);
        assert_eq!(m.cpu_read(0x8000), 0x1B, "block 4: 128KB at page $10");
        assert_eq!(m.cpu_read(0xE000), 0x1F);
        m.cpu_write(0x6000, 7);
        assert_eq!(m.cpu_read(0x8000), 0x1B);
        // CHR: block bit 2 picks the upper 128KB.
        set_reg(&mut m, 2, 5);
        assert_eq!(m.ppu_read(0x1000), 0x85);
        m.cpu_write(0x6000, 0);
        assert_eq!(m.ppu_read(0x1000), 5);
        // No work RAM: reads are open bus.
        assert_eq!(m.prg_ram_read(0x6000), None);
    }

    #[test]
    fn qj_halves_and_write_gate() {
        let mut m = board(Mmc3Board::Qj, 32, 256);
        set_reg(&mut m, 7, 0x12);
        assert_eq!(m.cpu_read(0xA000), 0x02);
        assert_eq!(m.cpu_read(0xE000), 0x0F);
        m.cpu_write(0x6000, 1);
        assert_eq!(m.cpu_read(0xA000), 0x12);
        assert_eq!(m.cpu_read(0xE000), 0x1F);
        set_reg(&mut m, 5, 0x83);
        assert_eq!(m.ppu_read(0x1C00), 0x83);
        // A write-protected $6000 doesn't reach the block register.
        m.cpu_write(0xA001, 0xC0);
        m.cpu_write(0x6000, 0);
        assert_eq!(m.cpu_read(0xE000), 0x1F);
    }

    #[test]
    fn tqrom_bit6_selects_chr_ram() {
        let mut m = board(Mmc3Board::Tqrom, 8, 64);
        set_reg(&mut m, 2, 0x05);
        assert_eq!(m.ppu_read(0x1000), 5);
        m.ppu_write(0x1000, 0xAA); // ROM: ignored
        assert_eq!(m.ppu_read(0x1000), 5);
        set_reg(&mut m, 2, 0x43);
        m.ppu_write(0x1000, 0xAA);
        assert_eq!(m.ppu_read(0x1000), 0xAA);
        set_reg(&mut m, 3, 0x43); // same RAM page through another slot
        assert_eq!(m.ppu_read(0x1400), 0xAA);
        set_reg(&mut m, 2, 0x44);
        assert_eq!(m.ppu_read(0x1000), 0, "different RAM page");
    }

    #[test]
    fn chr_ram_mixing_boards() {
        for (b, ram_bank, rom_bank) in [
            (Mmc3Board::ChrRam74, 8u8, 10u8),
            (Mmc3Board::ChrRam191, 0x81, 0x01),
            (Mmc3Board::ChrRam192, 11, 12),
            (Mmc3Board::ChrRam194, 1, 2),
            (Mmc3Board::ChrRam195, 3, 4),
        ] {
            let mut m = board(b, 8, 32);
            set_reg(&mut m, 2, ram_bank);
            m.ppu_write(0x1000, 0x5A);
            assert_eq!(m.ppu_read(0x1000), 0x5A, "{b:?} RAM bank {ram_bank}");
            set_reg(&mut m, 2, rom_bank);
            m.ppu_write(0x1000, 0x5A);
            assert_eq!(
                m.ppu_read(0x1000),
                rom_bank & 0x7F,
                "{b:?} ROM bank {rom_bank}"
            );
        }
    }

    #[test]
    fn board_savestate_roundtrip() {
        let mut m = board(Mmc3Board::Tqrom, 8, 64);
        set_reg(&mut m, 2, 0x41);
        m.ppu_write(0x1000, 0x77);
        let blob = m.save_state();
        let mut n = board(Mmc3Board::Tqrom, 8, 64);
        n.load_state(&blob).unwrap();
        assert_eq!(n.ppu_read(0x1000), 0x77);
        // A plain MMC3 state (no board RAM) must not load into TQROM.
        let plain = board(Mmc3Board::Standard, 8, 64).save_state();
        assert!(n.load_state(&plain).is_err());
    }

    #[test]
    fn prg_ram_read_write() {
        let mut m = mmc3();
        m.cpu_write(0x6123, 0xAB);
        assert_eq!(m.prg_ram_read(0x6123), Some(0xAB));
    }
}
