use crate::mapper::{Mapper, Mirroring, Nrom};

pub fn load_rom(data: &[u8]) -> Result<Box<dyn Mapper>, String> {
    if data.len() < 16 || &data[0..4] != b"NES\x1A" {
        return Err("not an iNES file (bad magic)".into());
    }
    let prg_banks = data[4] as usize;
    let chr_banks = data[5] as usize;
    let flags6 = data[6];
    let flags7 = data[7];
    let mapper_id = (flags6 >> 4) | (flags7 & 0xF0);
    let mirroring = if flags6 & 1 != 0 {
        Mirroring::Vertical
    } else {
        Mirroring::Horizontal
    };
    let has_trainer = flags6 & 0x04 != 0;

    if mapper_id != 0 {
        return Err(format!(
            "unsupported mapper {mapper_id} (only NROM/mapper 0)"
        ));
    }

    let prg_size = prg_banks * 16 * 1024;
    let chr_size = chr_banks * 8 * 1024;
    let prg_start = 16 + if has_trainer { 512 } else { 0 };
    let chr_start = prg_start + prg_size;

    if data.len() < chr_start + chr_size {
        return Err("ROM file truncated".into());
    }

    let prg = data[prg_start..prg_start + prg_size].to_vec();
    let chr = data[chr_start..chr_start + chr_size].to_vec();

    Ok(Box::new(Nrom::new(prg, chr, mirroring)))
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
        let mut mapper = load_rom(&data).unwrap();
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
}
