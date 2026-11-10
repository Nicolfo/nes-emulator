use super::Chr;
use super::discrete::{bank_index, chr_or_ram};
use super::flash::Sst39sf;
use super::{Mapper, Mirroring, NtTarget};
use serde::{Deserialize, Serialize};

/// GTROM / Cheapocabra (mapper 111), Membler Industries' homebrew board.
/// Register at $5000-$5FFF and $7000-$7FFF: bits 0-3 select a 32KB PRG
/// bank, bit 4 one of two 8KB CHR RAM banks, bit 5 one of two 4KB pages of
/// the board's 8KB nametable RAM (four-screen, all of $2000-$2FFF), bits 6-7
/// drive LEDs.
///
/// PRG is an SST39SF040 flash: writes to $8000-$FFFF reach it at
/// `bank * $8000 + (addr & $7FFF)`, so the unlock addresses are $D555 and
/// $AAAA in any bank. The flash image is the battery save when the header's
/// battery bit is set (see [`super::flash`]).
#[derive(Serialize, Deserialize)]
pub struct Gtrom {
    #[serde(default)]
    chip: Sst39sf,
    chr: Chr,
    chr_is_ram: bool,
    nt_ram: Vec<u8>,
    reg: u8,
}

impl Gtrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x4000);
        Gtrom {
            chip: Sst39sf::new(prg),
            chr: chr.into(),
            chr_is_ram,
            nt_ram: vec![0; 0x2000],
            reg: 0,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        bank_index(
            self.chr.len(),
            0x2000,
            ((self.reg >> 4) & 1) as usize,
            addr as usize,
        )
    }

    fn nt_offset(&self, addr: u16) -> usize {
        ((self.reg as usize >> 5) & 1) * 0x1000 + (addr as usize & 0x0FFF)
    }

    fn prg_index(&self, addr: u16) -> usize {
        bank_index(
            self.chip.len(),
            0x8000,
            (self.reg & 0x0F) as usize,
            addr as usize,
        )
    }
}

impl Mapper for Gtrom {
    fn save_state(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serialize mapper state")
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        let mut new: Self = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        super::StateField::restore(&mut new.chr, &mut self.chr, "chr")?;
        if new.nt_ram.len() != self.nt_ram.len() {
            return Err("savestate nametable RAM size doesn't match this ROM".into());
        }
        new.chip.graft(&mut self.chip)?;
        *self = new;
        Ok(())
    }

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram].into();
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.chip.read(self.prg_index(addr))
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x5000..=0x5FFF | 0x7000..=0x7FFF => self.reg = val,
            0x8000..=0xFFFF => {
                let i = self.prg_index(addr);
                self.chip.write(i, val);
            }
            _ => {}
        }
    }

    fn nt_target(&mut self, _addr: u16) -> NtTarget {
        NtTarget::Cart
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x2000 {
            self.nt_ram[self.nt_offset(addr)]
        } else {
            self.chr[self.chr_offset(addr)]
        }
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x2000 {
            let off = self.nt_offset(addr);
            self.nt_ram[off] = val;
        } else if self.chr_is_ram {
            let off = self.chr_offset(addr);
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        Mirroring::FourScreen
    }

    fn battery_save(&self) -> Option<&[u8]> {
        Some(self.chip.image())
    }

    fn battery_restore(&mut self, data: &[u8]) {
        self.chip.restore(data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::discrete::tests::banks;

    #[test]
    fn banks_and_nametable_pages() {
        let mut m = Gtrom::new(banks(32, 0x4000), vec![]);
        m.cpu_write(0x5000, 0x03);
        assert_eq!(m.cpu_read(0x8000), 6);
        m.cpu_write(0x8000, 0x0F); // not a register (a stray flash write)
        assert_eq!(m.cpu_read(0xC000), 7);
        // Four distinct nametables in page 0.
        for (i, nt) in [0x2000u16, 0x2400, 0x2800, 0x2C00].into_iter().enumerate() {
            assert_eq!(m.nt_target(nt), NtTarget::Cart);
            m.ppu_write(nt, i as u8 + 1);
        }
        assert_eq!(m.ppu_read(0x2C00), 4);
        m.cpu_write(0x7000, 0x20); // NT page 1
        assert_eq!(m.ppu_read(0x2000), 0);
        m.ppu_write(0x0005, 0x99);
        m.cpu_write(0x5000, 0x10); // CHR bank 1, NT page 0
        assert_eq!(m.ppu_read(0x0005), 0);
        assert_eq!(m.ppu_read(0x2400), 2);
    }

    fn unlock(m: &mut Gtrom) {
        m.cpu_write(0xD555, 0xAA);
        m.cpu_write(0xAAAA, 0x55);
    }

    #[test]
    fn self_flash_program_erase_id_and_persist() {
        let mut m = Gtrom::new(banks(32, 0x4000), vec![]);
        m.cpu_write(0x5000, 0x02); // bank 2: bytes 4 then 5
        unlock(&mut m);
        m.cpu_write(0xD555, 0x80);
        unlock(&mut m);
        m.cpu_write(0xC000, 0x30); // erase bank 2's sector at $C000
        assert_eq!(m.cpu_read(0xC000), 0xFF);
        assert_eq!(m.cpu_read(0xD000), 5, "next sector untouched");
        unlock(&mut m);
        m.cpu_write(0xD555, 0xA0);
        m.cpu_write(0xC010, 0x12);
        assert_eq!(m.cpu_read(0xC010), 0x12);

        unlock(&mut m);
        m.cpu_write(0xD555, 0x90);
        assert_eq!((m.cpu_read(0x8000), m.cpu_read(0x8001)), (0xBF, 0xB7));
        m.cpu_write(0x8000, 0xF0);
        assert_eq!(m.cpu_read(0x8000), 4);

        let sav = m.battery_save().unwrap().to_vec();
        let state = m.save_state();
        let mut fresh = Gtrom::new(banks(32, 0x4000), vec![]);
        fresh.battery_restore(&sav);
        fresh.cpu_write(0x5000, 0x02);
        assert_eq!(fresh.cpu_read(0xC010), 0x12, ".sav round trip");

        let mut other = Gtrom::new(banks(32, 0x4000), vec![]);
        other.load_state(&state).unwrap();
        assert_eq!(other.cpu_read(0xC010), 0x12, "savestate round trip");
        assert_eq!(other.cpu_read(0xD000), 5);
    }
}
