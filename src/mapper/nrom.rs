use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Nrom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
}

impl Nrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Nrom {
            prg,
            chr,
            chr_is_ram,
            mirroring,
        }
    }
}

impl Mapper for Nrom {
    crate::impl_mapper_savestate!();
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            // mask handles both 16KB (mirrored) and 32KB PRG
            self.prg[(addr as usize - 0x8000) & (self.prg.len() - 1)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, _addr: u16, _val: u8) {}

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
