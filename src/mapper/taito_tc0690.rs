use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Taito TC0190 / TC0690 (mappers 33 and 48). The two boards share their PRG
/// and CHR banking; the TC0690 (48) adds an MMC3-style A12 scanline IRQ and
/// moves the mirroring control. This one type serves both:
///
/// ```text
/// $8000  [.MPP PPPP]  PRG Reg 0 (8KB @ $8000); bit 6 = mirroring on TC0190
/// $8001  [..PP PPPP]  PRG Reg 1 (8KB @ $A000)
/// $8002  [CCCC CCCC]  CHR Reg 0 (2KB @ $0000)
/// $8003  [CCCC CCCC]  CHR Reg 1 (2KB @ $0800)
/// $A000-$A003          CHR Regs 2-5 (1KB @ $1000/$1400/$1800/$1C00)
/// --- TC0690 (mapper 48) only ---
/// $C000  [IIII IIII]  IRQ latch (reload value, stored one's-complemented)
/// $C001                IRQ reload
/// $C002                IRQ enable
/// $C003                IRQ disable + acknowledge
/// $E000  [.M.. ....]  M = mirroring (0=Vert, 1=Horz)
/// ```
///
/// Most dumps of the six TC0690 games (Flintstones - Rescue of Dino & Hoppy,
/// Don Doko Don 2, Bubble Bobble 2 (J), Captain Saver (J), Jetsons (J),
/// Bakushou!! Jinsei Gekijou 3) are mislabelled as mapper 33. A real TC0190
/// has no registers above $BFFF, so the first CPU write to $C000-$FFFF is
/// proof the board is actually a TC0690: we promote to TC0690 behaviour
/// (IRQ + $E000 mirroring) on that write. Games that wait on the IRQ during
/// init (Flintstones) then boot instead of hanging. Explicit mapper-48 ROMs
/// start already promoted. Mirroring follows $8000 bit 6 until promotion and
/// $E000 afterwards.
///
/// The IRQ counter is clocked exactly like the MMC3's (rising edges on PPU
/// A12); the one TC0690 quirk is that the value written to $C000 is inverted
/// (one's-complemented) before use as the reload value (per the nesdev wiki:
/// "XOR the writes with $FF and it will work just like MMC3").
#[derive(Serialize, Deserialize)]
pub struct TaitoTc0690 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    four_screen: bool,
    /// False = TC0190 (mapper 33, no IRQ, mirroring at $8000 bit 6). Set true
    /// for an explicit mapper-48 ROM, or auto-set when a mapper-33 ROM writes
    /// the TC0690 register space at $C000-$FFFF.
    tc0690_mode: bool,
    prg_regs: [u8; 2],
    chr_2k: [u8; 2],
    chr_1k: [u8; 4],
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_line: bool,
    last_a12: bool,
}

impl TaitoTc0690 {
    pub fn new(mapper_id: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        let four_screen = mirroring == Mirroring::FourScreen;
        TaitoTc0690 {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            four_screen,
            tc0690_mode: mapper_id == 48,
            prg_regs: [0; 2],
            chr_2k: [0; 2],
            chr_1k: [0; 4],
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_line: false,
            last_a12: false,
        }
    }

    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x2000;
        let last = banks - 1;
        let bank = match addr {
            0x8000..=0x9FFF => self.prg_regs[0] as usize % banks,
            0xA000..=0xBFFF => self.prg_regs[1] as usize % banks,
            0xC000..=0xDFFF => last - 1,
            _ => last,
        };
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        match addr >> 10 {
            0 | 1 => {
                let bank = (self.chr_2k[0] as usize * 2 + (addr as usize >> 10 & 1)) % banks;
                bank * 0x400 + (addr as usize & 0x3FF)
            }
            2 | 3 => {
                let bank = (self.chr_2k[1] as usize * 2 + (addr as usize >> 10 & 1)) % banks;
                bank * 0x400 + (addr as usize & 0x3FF)
            }
            k => {
                let bank = self.chr_1k[k as usize - 4] as usize % banks;
                bank * 0x400 + (addr as usize & 0x3FF)
            }
        }
    }

    /// Clock the IRQ counter on each A12 rising edge, MMC3-style.
    ///
    /// Hardware asserts the IRQ ~4 CPU cycles later than the MMC3 does
    /// (per the nesdev wiki). That delay is sub-scanline (~12 PPU dots, vs
    /// ~341 per line), so the split lands on the same scanline and no game's
    /// output changes; we assert immediately, matching this emulator's MMC3.
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

impl Mapper for TaitoTc0690 {
    crate::impl_mapper_savestate!(prg, chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
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
        // A real TC0190 (mapper 33) has no registers above $BFFF, so a write
        // there means this "mapper 33" ROM is really a TC0690.
        if addr >= 0xC000 {
            self.tc0690_mode = true;
        }
        match addr & 0xE003 {
            0x8000 => {
                self.prg_regs[0] = val & 0x3F;
                // TC0190 carries mirroring in $8000 bit 6; the TC0690 uses
                // $8000 solely for PRG and moved mirroring to $E000. Honour
                // bit 6 only while still in TC0190 mode.
                if !self.tc0690_mode && !self.four_screen {
                    self.mirroring = if val & 0x40 != 0 {
                        Mirroring::Horizontal
                    } else {
                        Mirroring::Vertical
                    };
                }
            }
            0x8001 => self.prg_regs[1] = val & 0x3F,
            0x8002 => self.chr_2k[0] = val,
            0x8003 => self.chr_2k[1] = val,
            0xA000 => self.chr_1k[0] = val,
            0xA001 => self.chr_1k[1] = val,
            0xA002 => self.chr_1k[2] = val,
            0xA003 => self.chr_1k[3] = val,
            0xC000 => self.irq_latch = val ^ 0xFF,
            0xC001 => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            0xC002 => self.irq_enabled = true,
            0xC003 => {
                self.irq_enabled = false;
                self.irq_line = false;
            }
            0xE000 if !self.four_screen => {
                self.mirroring = if val & 0x40 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.watch_a12(addr);
        self.chr[self.chr_offset(addr)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        self.watch_a12(addr);
        if self.chr_is_ram {
            let off = self.chr_offset(addr);
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn irq(&self) -> bool {
        self.irq_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(mapper_id: u8) -> TaitoTc0690 {
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        TaitoTc0690::new(mapper_id, prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_banking() {
        let mut m = board(48);
        m.cpu_write(0x8000, 3);
        m.cpu_write(0x8001, 4);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xA000), 4);
        assert_eq!(m.cpu_read(0xC000), 6); // second-last fixed
        assert_eq!(m.cpu_read(0xE000), 7); // last fixed
    }

    #[test]
    fn chr_2k_and_1k_banks() {
        let mut m = board(48);
        m.cpu_write(0x8002, 1); // 2KB @ $0000 -> 1KB banks 2,3
        m.cpu_write(0x8003, 2); // 2KB @ $0800 -> 1KB banks 4,5
        m.cpu_write(0xA000, 9); // 1KB @ $1000
        assert_eq!(m.ppu_read(0x0000), 2);
        assert_eq!(m.ppu_read(0x0400), 3);
        assert_eq!(m.ppu_read(0x0800), 4);
        assert_eq!(m.ppu_read(0x1000), 9);
    }

    #[test]
    fn tc0190_mirroring_via_8000_bit6() {
        // Mapper 33: mirroring lives in $8000 bit 6, no promotion.
        let mut m = board(33);
        m.cpu_write(0x8000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0x8000, 0x40);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn tc0190_has_no_irq() {
        // Without any $C000+ write the board stays a plain TC0190: the IRQ
        // never fires even as A12 toggles.
        let mut m = board(33);
        for _ in 0..10 {
            m.ppu_read(0x0000);
            m.ppu_read(0x1000);
        }
        assert!(!m.irq());
    }

    #[test]
    fn tc0690_mirroring_on_e000() {
        let mut m = board(48);
        m.cpu_write(0xE000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0xE000, 0x40);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn irq_counts_a12_rises() {
        let mut m = board(48);
        m.cpu_write(0xC000, 0xFF ^ 3); // latch = 3 (stored inverted)
        m.cpu_write(0xC001, 0); // reload
        m.cpu_write(0xC002, 0); // enable
        for i in 0..3 {
            m.ppu_read(0x0000);
            m.ppu_read(0x1000); // reload to 3, then 2, 1
            assert!(!m.irq(), "IRQ too early at clock {i}");
        }
        m.ppu_read(0x0000);
        m.ppu_read(0x1000); // hits 0
        assert!(m.irq());
        m.cpu_write(0xC003, 0); // disable + ack
        assert!(!m.irq());
    }

    #[test]
    fn mapper33_dump_promotes_on_irq_write() {
        // A mislabelled mapper-33 ROM that drives the IRQ registers is treated
        // as a TC0690: the IRQ becomes live and mirroring follows $E000.
        let mut m = board(33);
        m.cpu_write(0x8000, 0x40); // would be Horizontal on a true TC0190
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        // Game sets up the scanline IRQ (this $C000 write triggers promotion).
        m.cpu_write(0xC000, 0xFF ^ 2); // latch = 2
        m.cpu_write(0xC001, 0); // reload
        m.cpu_write(0xC002, 0); // enable
        // After promotion, $8000 bit 6 no longer touches mirroring...
        m.cpu_write(0x8000, 0x40);
        // ...and $E000 controls it.
        m.cpu_write(0xE000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        // IRQ now fires (latch 2: reload, then 2 -> 1 -> 0 over three rises).
        for _ in 0..3 {
            m.ppu_read(0x0000);
            m.ppu_read(0x1000);
        }
        assert!(m.irq());
    }

    #[test]
    fn four_screen_ignores_both_mirror_paths() {
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        let mut m = TaitoTc0690::new(48, prg, chr, Mirroring::FourScreen);
        m.cpu_write(0x8000, 0x40);
        m.cpu_write(0xE000, 0x40);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }
}
