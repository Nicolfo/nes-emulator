use crate::mapper::{
    Action53, Axrom, Bnrom, Cnrom, Codemasters, ColorDreams, Fme7, Gxrom, HolyDiver, Mapper,
    Mirroring, Mmc1, Mmc2, Mmc3, Mmc4, Mmc5, N163, Namco108, Nrom, Txsrom, Unrom180, Uxrom, Vrc4,
    Vrc6,
};

/// TV system the cartridge targets; drives CPU/PPU clock ratio, frame
/// layout, APU timing and frame pacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Region {
    Ntsc,
    Pal,
}

/// `battery` is the iNES flags6 bit 1: the board has battery-backed PRG RAM
/// that should persist to a .sav file.
pub fn load_rom(data: &[u8]) -> Result<(Box<dyn Mapper>, Region, bool), String> {
    if data.len() < 16 || &data[0..4] != b"NES\x1A" {
        return Err("not an iNES file (bad magic)".into());
    }
    let prg_banks = data[4] as usize;
    let chr_banks = data[5] as usize;
    let flags6 = data[6];
    let flags7 = data[7];
    let mapper_id = (flags6 >> 4) | (flags7 & 0xF0);
    let region = if flags7 & 0x0C == 0x08 {
        // NES 2.0: timing byte (Dendy and multi-region fall back to NTSC).
        if data[12] & 3 == 1 {
            Region::Pal
        } else {
            Region::Ntsc
        }
    } else if data[9] & 1 != 0 {
        // Legacy iNES TV-system bit; rarely set but free to honor.
        Region::Pal
    } else {
        Region::Ntsc
    };
    // flags6 bit 3 is the four-screen pad: the cartridge supplies its own
    // nametable RAM and all four nametables are distinct, overriding the
    // horizontal/vertical bit (and any mapper mirroring register).
    let mirroring = if flags6 & 0x08 != 0 {
        Mirroring::FourScreen
    } else if flags6 & 1 != 0 {
        Mirroring::Vertical
    } else {
        Mirroring::Horizontal
    };
    let has_trainer = flags6 & 0x04 != 0;
    let battery = flags6 & 0x02 != 0;

    let prg_size = prg_banks * 16 * 1024;
    let chr_size = chr_banks * 8 * 1024;
    let prg_start = 16 + if has_trainer { 512 } else { 0 };
    let chr_start = prg_start + prg_size;

    if data.len() < chr_start + chr_size {
        return Err("ROM file truncated".into());
    }

    let prg = data[prg_start..prg_start + prg_size].to_vec();
    let chr = data[chr_start..chr_start + chr_size].to_vec();

    let mapper: Box<dyn Mapper> = match mapper_id {
        0 => Box::new(Nrom::new(prg, chr, mirroring)),
        1 => Box::new(Mmc1::new(prg, chr)), // mirroring register-controlled
        2 => Box::new(Uxrom::new(prg, chr, mirroring)),
        3 => Box::new(Cnrom::new(prg, chr, mirroring)),
        4 => Box::new(Mmc3::new(prg, chr, mirroring)),
        5 => Box::new(Mmc5::new(prg, chr, mirroring)),
        7 => Box::new(Axrom::new(prg, chr)), // single-screen, register-controlled
        9 => Box::new(Mmc2::new(prg, chr, mirroring)),
        10 => Box::new(Mmc4::new(prg, chr, mirroring)),
        11 => Box::new(ColorDreams::new(prg, chr, mirroring)),
        19 => Box::new(N163::new(prg, chr, mirroring)),
        21 | 22 | 23 | 25 => Box::new(Vrc4::new(mapper_id, prg, chr, mirroring)),
        24 => Box::new(Vrc6::new(prg, chr, mirroring)),
        26 => Box::new(Vrc6::new_vrc6b(prg, chr, mirroring)),
        28 => Box::new(Action53::new(prg, chr, mirroring)),
        34 => Box::new(Bnrom::new(prg, chr, mirroring)),
        66 => Box::new(Gxrom::new(prg, chr, mirroring)),
        69 => Box::new(Fme7::new(prg, chr, mirroring)),
        71 => Box::new(Codemasters::new(prg, chr, mirroring)),
        78 => Box::new(HolyDiver::new(prg, chr, mirroring)),
        118 => Box::new(Txsrom::new(prg, chr, mirroring)),
        180 => Box::new(Unrom180::new(prg, chr, mirroring)),
        206 => Box::new(Namco108::new(prg, chr, mirroring)),
        _ => return Err(format!("mapper {mapper_id} is not supported")),
    };
    Ok((mapper, region, battery))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nestest_header() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nestest.nes");
        let data = std::fs::read(path).unwrap();
        assert_eq!(&data[0..4], b"NES\x1A");
        assert_eq!(data[4], 1); // 16KB PRG
        assert_eq!(data[5], 1); // 8KB CHR
        // We'll check the mirroring and mapper
        assert_eq!((data[6] >> 4) | (data[7] & 0xF0), 0); // mapper 0
        let (mut mapper, region, _) = load_rom(&data).unwrap();
        assert_eq!(region, Region::Ntsc);
        // Let's assert reset vector points to PRG space (>= 0x8000)
        let lo = mapper.cpu_read(0xFFFC) as u16;
        let hi = mapper.cpu_read(0xFFFD) as u16;
        let reset = (hi << 8) | lo;
        assert!(reset >= 0x8000);
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(load_rom(&[0u8; 32]).is_err());
    }

    /// Minimal 32KB mapper-0 image; `flags6` carries the mirroring bits.
    fn rom(flags6: u8) -> Vec<u8> {
        let mut data = vec![0u8; 16 + 32 * 1024 + 8 * 1024];
        data[0..4].copy_from_slice(b"NES\x1A");
        data[4] = 2; // 2x 16KB PRG
        data[5] = 1; // 8KB CHR
        data[6] = flags6;
        data
    }

    #[test]
    fn four_screen_flag_sets_four_screen_mirroring() {
        // flags6 bit 3 set; bit 0 (vertical) must be overridden.
        let (mapper, _, _) = load_rom(&rom(0x09)).unwrap();
        assert_eq!(mapper.mirroring(), Mirroring::FourScreen);
    }

    #[test]
    fn without_four_screen_flag_uses_mirroring_bit() {
        let (mapper, _, _) = load_rom(&rom(0x01)).unwrap();
        assert_eq!(mapper.mirroring(), Mirroring::Vertical);
        let (mapper, _, _) = load_rom(&rom(0x00)).unwrap();
        assert_eq!(mapper.mirroring(), Mirroring::Horizontal);
    }
}
