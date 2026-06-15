use crate::mapper::{
    Action53, Axrom, BandaiFcg, Bnrom, Cnrom, Codemasters, ColorDreams, Fme7, Gxrom, H3001,
    HolyDiver, Mapper, Mirroring, Mmc1, Mmc2, Mmc3, Mmc4, Mmc5, N163, Namco108, Namco175340, Nrom,
    Rambo1, Sunsoft4, Txsrom, Unrom180, Uxrom, Vrc1, Vrc3, Vrc4, Vrc6,
};

/// TV system the cartridge targets; drives CPU/PPU clock ratio, frame
/// layout, APU timing and frame pacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Region {
    Ntsc,
    Pal,
}

/// Decodes an iNES/NES 2.0 ROM-area size in bytes. `lsb` is the byte-4 (PRG)
/// or byte-5 (CHR) count; `msb` is the NES 2.0 4-bit extension from byte 9 (0
/// for plain iNES). When the 12-bit value's top nibble is 0xF the size uses
/// NES 2.0's exponent-multiplier form (`2^E * (2*M+1)` bytes), which encodes
/// sizes that aren't a whole number of `unit`-sized banks; otherwise it is the
/// `(msb:lsb)` bank count times `unit`. `saturating_mul` keeps a bogus header
/// from overflowing - the caller's length check then rejects it cleanly.
fn rom_size(lsb: u8, msb: u8, unit: usize) -> usize {
    if msb == 0x0F {
        let exp = (lsb >> 2) as u32;
        let mult = (2 * (lsb & 0x03) + 1) as usize;
        (1usize << exp).saturating_mul(mult)
    } else {
        (((msb as usize) << 8) | lsb as usize) * unit
    }
}

/// `battery` is the iNES flags6 bit 1: the board has battery-backed PRG RAM
/// that should persist to a .sav file.
pub fn load_rom(data: &[u8]) -> Result<(Box<dyn Mapper>, Region, bool), String> {
    if data.len() < 16 || &data[0..4] != b"NES\x1A" {
        return Err("not an iNES file (bad magic)".into());
    }
    let flags6 = data[6];
    let flags7 = data[7];
    let mapper_id = (flags6 >> 4) | (flags7 & 0xF0);
    // NES 2.0 (flags7 bits 2-3 = %10) carries a submapper in the high nibble of
    // byte 8; plain iNES has none, so it reads as 0 ("unspecified"). Mappers
    // that share a number across distinct boards (e.g. Bandai 16, Namco 210)
    // use it to pick the exact hardware.
    let nes2 = flags7 & 0x0C == 0x08;
    let submapper = if nes2 { data[8] >> 4 } else { 0 };
    // PRG/CHR ROM sizes. NES 2.0 widens each size with a 4-bit MSB in byte 9
    // (low nibble = PRG, high nibble = CHR); plain iNES has no MSB, so the
    // nibble is 0 and the size is just the byte-4/5 bank count. See `rom_size`
    // for the exponent-multiplier form NES 2.0 uses when the nibble is 0xF.
    let prg_size = rom_size(data[4], if nes2 { data[9] & 0x0F } else { 0 }, 16 * 1024);
    let chr_size = rom_size(data[5], if nes2 { data[9] >> 4 } else { 0 }, 8 * 1024);
    let region = if nes2 {
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
        16 | 159 => Box::new(BandaiFcg::new(mapper_id, submapper, prg, chr, mirroring)),
        19 => Box::new(N163::new(prg, chr, mirroring)),
        21 | 22 | 23 | 25 => Box::new(Vrc4::new(mapper_id, prg, chr, mirroring)),
        24 => Box::new(Vrc6::new(prg, chr, mirroring)),
        26 => Box::new(Vrc6::new_vrc6b(prg, chr, mirroring)),
        28 => Box::new(Action53::new(prg, chr, mirroring)),
        34 => Box::new(Bnrom::new(prg, chr, mirroring)),
        64 => Box::new(Rambo1::new(prg, chr, mirroring)),
        65 => Box::new(H3001::new(prg, chr, mirroring)),
        66 => Box::new(Gxrom::new(prg, chr, mirroring)),
        68 => Box::new(Sunsoft4::new(prg, chr, mirroring)),
        69 => Box::new(Fme7::new(prg, chr, mirroring)),
        71 => Box::new(Codemasters::new(prg, chr, mirroring)),
        73 => Box::new(Vrc3::new(prg, chr, mirroring)),
        75 => Box::new(Vrc1::new(prg, chr, mirroring)),
        78 => Box::new(HolyDiver::new(prg, chr, mirroring)),
        118 => Box::new(Txsrom::new(prg, chr, mirroring)),
        180 => Box::new(Unrom180::new(prg, chr, mirroring)),
        206 => Box::new(Namco108::new(prg, chr, mirroring)),
        210 => Box::new(Namco175340::new(submapper, prg, chr, mirroring)),
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

    #[test]
    fn rom_size_decodes_ines_and_nes2_forms() {
        // Plain iNES: bank count * unit, no MSB.
        assert_eq!(rom_size(1, 0, 16 * 1024), 16 * 1024);
        assert_eq!(rom_size(2, 0, 8 * 1024), 16 * 1024);
        // NES 2.0 MSB nibble extends the count into the 0x100+ bank range.
        assert_eq!(rom_size(0, 1, 16 * 1024), 0x100 * 16 * 1024);
        assert_eq!(rom_size(0x34, 1, 16 * 1024), 0x134 * 16 * 1024);
        // Exponent-multiplier form (top nibble 0xF): size = 2^E * (2*M+1) bytes,
        // independent of `unit`.
        assert_eq!(rom_size(0b0000_0000, 0x0F, 16 * 1024), 1); // E=0, M=0
        assert_eq!(rom_size(0b0000_1000, 0x0F, 16 * 1024), 4); // E=2, M=0
        assert_eq!(rom_size(0b0000_0001, 0x0F, 16 * 1024), 3); // E=0, M=1
    }

    #[test]
    fn nes2_chr_size_msb_is_honored() {
        // NES 2.0 header (flags7 bits 2-3 = %10) with a CHR-ROM MSB nibble of 1
        // means 0x101 CHR banks (~2 MB); a small file must be rejected as
        // truncated, proving byte 9's high nibble feeds the size. Without the
        // MSB the very same file parses.
        let mut data = vec![0u8; 16 + 16 * 1024 + 8 * 1024];
        data[0..4].copy_from_slice(b"NES\x1A");
        data[4] = 1; // 1x 16KB PRG
        data[5] = 1; // CHR LSB = 1
        data[7] = 0x08; // NES 2.0, mapper 0
        data[9] = 0x10; // CHR-size MSB nibble = 1
        assert!(load_rom(&data).is_err());
        data[9] = 0x00;
        assert!(load_rom(&data).is_ok());
    }
}
