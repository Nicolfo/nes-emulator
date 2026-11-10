use super::discrete::{bank_index, chr_or_ram};
use super::flash::Sst39sf;
use super::{Mapper, Mirroring, NtTarget};
use serde::{Deserialize, Serialize};

/// How the board wires the nametables, from the header's mirroring bits
/// (flags 6 bits 0 and 3) and the submapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum NtMode {
    Fixed(Mirroring),
    /// Register bit 7 picks the single-screen page.
    OneScreen,
    /// Submapper 3: register bit 7 picks horizontal (0) / vertical (1).
    Switchable,
}

/// UNROM 512 (mapper 30), the RetroUSB/InfiniteNESLives homebrew board:
/// register `[MCCP PPPP]` - 16KB PRG bank at $8000 (last bank fixed at
/// $C000), 8KB CHR RAM bank out of 32KB, and bit 7 the one-screen page on
/// single-screen boards.
///
/// The self-flashable variant (header battery bit set) carries an
/// SST39SF0x0: the register only answers at $C000-$FFFF, and writes to
/// $8000-$BFFF go to the flash at `bank * $4000 + (addr & $3FFF)` - so the
/// unlock addresses $5555/$2AAA are reached as bank 1:$9555 and bank 0:$AAAA.
/// The flash image is the battery save (see [`super::flash`]).
///
/// Four-screen boards (flags 6 bits 0 and 3 both set) use the last 8KB of
/// the 32KB CHR RAM as nametable RAM for $2000-$3EFF.
#[derive(Serialize, Deserialize)]
pub struct Unrom512 {
    /// PRG flash (or plain ROM on non-flashable boards: writes never reach it).
    #[serde(default)]
    chip: Sst39sf,
    chr: Vec<u8>,
    chr_is_ram: bool,
    #[serde(alias = "flash")]
    flashable: bool,
    nt: NtMode,
    reg: u8,
}

impl Unrom512 {
    /// `header_nt` is flags 6 & $09 (bit 0 vertical, bit 3 four-screen
    /// pad); `battery` marks the self-flashable board.
    pub fn new(submapper: u8, header_nt: u8, battery: bool, prg: Vec<u8>, chr: Vec<u8>) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x8000);
        let nt = if submapper == 3 {
            NtMode::Switchable
        } else {
            match header_nt & 0x09 {
                0x00 => NtMode::Fixed(Mirroring::Horizontal),
                0x01 => NtMode::Fixed(Mirroring::Vertical),
                0x08 => NtMode::OneScreen,
                _ => NtMode::Fixed(Mirroring::FourScreen),
            }
        };
        Unrom512 {
            chip: Sst39sf::new(prg),
            chr,
            chr_is_ram,
            flashable: battery,
            nt,
            reg: 0,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        bank_index(
            self.chr.len(),
            0x2000,
            ((self.reg >> 5) & 3) as usize,
            addr as usize,
        )
    }

    fn prg_index(&self, bank: usize, addr: u16) -> usize {
        bank_index(self.chip.len(), 0x4000, bank, addr as usize)
    }

    /// Four-screen nametables live in the last 8KB of CHR RAM (when the
    /// board has the full 32KB; otherwise the console's four-screen RAM).
    fn nt_in_chr_ram(&self) -> bool {
        self.nt == NtMode::Fixed(Mirroring::FourScreen)
            && self.chr_is_ram
            && self.chr.len() >= 0x8000
    }

    fn nt_offset(&self, addr: u16) -> usize {
        self.chr.len() - 0x2000 + (addr as usize & 0x1FFF)
    }
}

impl Mapper for Unrom512 {
    fn save_state(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serialize mapper state")
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        let mut new: Self = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        if new.chr.len() != self.chr.len() {
            return Err(format!(
                "savestate chr is {} bytes; this ROM uses {}",
                new.chr.len(),
                self.chr.len()
            ));
        }
        new.chip.graft(&mut self.chip)?;
        *self = new;
        Ok(())
    }

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let bank = if addr < 0xC000 {
            (self.reg & 0x1F) as usize
        } else {
            (self.chip.len() / 0x4000).saturating_sub(1)
        };
        self.chip.read(self.prg_index(bank, addr))
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if self.flashable {
            match addr {
                0x8000..=0xBFFF => {
                    let i = self.prg_index((self.reg & 0x1F) as usize, addr);
                    self.chip.write(i, val);
                }
                0xC000..=0xFFFF => self.reg = val,
                _ => {}
            }
        } else if addr >= 0x8000 {
            self.reg = val;
        }
    }

    fn nt_target(&mut self, addr: u16) -> NtTarget {
        if self.nt_in_chr_ram() {
            NtTarget::Cart
        } else {
            NtTarget::Ciram(super::mirror_nt(self.mirroring(), addr))
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x2000 {
            return self.chr[self.nt_offset(addr)];
        }
        self.chr[self.chr_offset(addr)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x2000 {
            let off = self.nt_offset(addr);
            self.chr[off] = val;
        } else if self.chr_is_ram {
            let off = self.chr_offset(addr);
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        let bit7 = self.reg & 0x80 != 0;
        match self.nt {
            NtMode::Fixed(m) => m,
            NtMode::OneScreen if bit7 => Mirroring::SingleScreenHi,
            NtMode::OneScreen => Mirroring::SingleScreenLo,
            NtMode::Switchable if bit7 => Mirroring::Vertical,
            NtMode::Switchable => Mirroring::Horizontal,
        }
    }

    fn battery_save(&self) -> Option<&[u8]> {
        self.flashable.then(|| self.chip.image())
    }

    fn battery_restore(&mut self, data: &[u8]) {
        if self.flashable {
            self.chip.restore(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::discrete::tests::banks;

    #[test]
    fn prg_and_chr_ram_banks() {
        let mut m = Unrom512::new(0, 0x01, false, banks(32, 0x4000), vec![]);
        m.cpu_write(0x8000, 0b0110_0101);
        assert_eq!(m.cpu_read(0x8000), 5);
        assert_eq!(m.cpu_read(0xFFFF), 31, "last bank fixed");
        m.ppu_write(0x0010, 0xAB); // CHR bank 3
        m.cpu_write(0x8000, 0);
        assert_eq!(m.ppu_read(0x0010), 0);
        m.cpu_write(0x8000, 0x60);
        assert_eq!(m.ppu_read(0x0010), 0xAB);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        assert!(m.battery_save().is_none(), "non-flash board saves nothing");
    }

    #[test]
    fn one_screen_and_flash_variants() {
        let mut m = Unrom512::new(0, 0x08, true, banks(32, 0x4000), vec![]);
        m.cpu_write(0x8000, 0x83); // flash command space: not the register
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0xC000, 0x83);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);

        let mut s3 = Unrom512::new(3, 0x00, false, banks(2, 0x4000), vec![]);
        s3.cpu_write(0x8000, 0x80);
        assert_eq!(s3.mirroring(), Mirroring::Vertical);
    }

    /// The documented UNROM 512 write sequence: select the bank for each
    /// command write through $C000, then write to $8000-$BFFF.
    fn flash_cmd(m: &mut Unrom512, seq: &[(u8, u16, u8)]) {
        for &(bank, addr, val) in seq {
            m.cpu_write(0xC000, bank);
            m.cpu_write(addr, val);
        }
    }

    fn unlock() -> [(u8, u16, u8); 2] {
        [(1, 0x9555, 0xAA), (0, 0xAAAA, 0x55)]
    }

    fn flash_board() -> Unrom512 {
        Unrom512::new(0, 0x01, true, banks(32, 0x4000), vec![])
    }

    #[test]
    fn self_flash_program_erase_and_id() {
        let mut m = flash_board();
        // Erase the sector at bank 4 offset $1000, then program a byte.
        flash_cmd(&mut m, &unlock());
        flash_cmd(&mut m, &[(1, 0x9555, 0x80)]);
        flash_cmd(&mut m, &unlock());
        flash_cmd(&mut m, &[(4, 0x9000, 0x30)]);
        m.cpu_write(0xC000, 4);
        assert_eq!(m.cpu_read(0x9000), 0xFF);
        assert_eq!(m.cpu_read(0x8000), 4, "other sector untouched");
        flash_cmd(&mut m, &unlock());
        flash_cmd(&mut m, &[(1, 0x9555, 0xA0), (4, 0x9123, 0x5A)]);
        m.cpu_write(0xC000, 4);
        assert_eq!(m.cpu_read(0x9123), 0x5A);

        // Software ID.
        flash_cmd(&mut m, &unlock());
        flash_cmd(&mut m, &[(1, 0x9555, 0x90)]);
        assert_eq!(m.cpu_read(0x8000), 0xBF);
        assert_eq!(m.cpu_read(0x8001), 0xB7);
        flash_cmd(&mut m, &[(0, 0x8000, 0xF0)]);
        assert_eq!(m.cpu_read(0x8000), 0);
    }

    #[test]
    fn non_flash_board_never_writes_prg() {
        let mut m = Unrom512::new(0, 0x01, false, banks(32, 0x4000), vec![]);
        flash_cmd(&mut m, &unlock());
        flash_cmd(&mut m, &[(1, 0x9555, 0xA0), (0, 0x8000, 0x00)]);
        m.cpu_write(0x8000, 3);
        assert_eq!(m.cpu_read(0x8000), 3, "plain ROM: writes are the register");
    }

    #[test]
    fn flash_persists_through_battery_hooks() {
        let mut m = flash_board();
        flash_cmd(&mut m, &unlock());
        flash_cmd(&mut m, &[(1, 0x9555, 0xA0), (7, 0x8042, 0x00)]);
        let sav = m.battery_save().unwrap().to_vec();
        assert_eq!(sav.len(), 32 * 0x4000);

        let mut fresh = flash_board();
        fresh.battery_restore(&[0u8; 17]); // wrong size: ignored
        fresh.cpu_write(0xC000, 7);
        assert_eq!(fresh.cpu_read(0x8042), 7);
        fresh.battery_restore(&sav);
        assert_eq!(fresh.cpu_read(0x8042), 0);
    }

    #[test]
    fn savestate_keeps_flash_writes() {
        let mut m = flash_board();
        flash_cmd(&mut m, &unlock());
        flash_cmd(&mut m, &[(1, 0x9555, 0xA0), (9, 0x8001, 0x00)]);
        let state = m.save_state();
        // A write after the snapshot must be undone by loading it.
        flash_cmd(&mut m, &unlock());
        flash_cmd(&mut m, &[(1, 0x9555, 0xA0), (2, 0x8000, 0x00)]);
        m.load_state(&state).unwrap();
        m.cpu_write(0xC000, 9);
        assert_eq!(m.cpu_read(0x8001), 0);
        m.cpu_write(0xC000, 2);
        assert_eq!(m.cpu_read(0x8000), 2);
        // Clean boards' states carry no PRG data.
        assert!(flash_board().save_state().len() < 2 * 0x8000 * 4);
    }

    #[test]
    fn four_screen_uses_last_8k_of_chr_ram() {
        let mut m = Unrom512::new(0, 0x09, false, banks(2, 0x4000), vec![]);
        for (i, nt) in [0x2000u16, 0x2400, 0x2800, 0x2C00].into_iter().enumerate() {
            assert_eq!(m.nt_target(nt), NtTarget::Cart);
            m.ppu_write(nt, i as u8 + 1);
        }
        assert_eq!(m.ppu_read(0x2C00), 4);
        // Same bytes as CHR bank 3 at $0000.
        m.cpu_write(0x8000, 0x60);
        assert_eq!(m.ppu_read(0x0400), 2);
        // Other mirrorings stay on the console's CIRAM.
        let mut v = Unrom512::new(0, 0x01, false, banks(2, 0x4000), vec![]);
        assert!(matches!(v.nt_target(0x2400), NtTarget::Ciram(_)));
    }
}
