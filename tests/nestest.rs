//! CPU validation against the nestest golden log.
//! Covers all official opcodes plus the unofficial-NOP section
//! (log lines 1..=5259; line 5260 starts LAX/SAX/... which we don't implement).

use nes_emulator::nes::Nes;

struct Expected {
    pc: u16,
    a: u8,
    x: u8,
    y: u8,
    p: u8,
    sp: u8,
    cyc: u64,
}

fn parse_line(line: &str) -> Expected {
    fn hex8(line: &str, tag: &str) -> u8 {
        let i = line.find(tag).unwrap() + tag.len();
        u8::from_str_radix(&line[i..i + 2], 16).unwrap()
    }
    let pc = u16::from_str_radix(&line[0..4], 16).unwrap();
    let cyc_pos = line.find("CYC:").unwrap() + 4;
    let cyc = line[cyc_pos..].trim().parse().unwrap();
    Expected {
        pc,
        a: hex8(line, "A:"),
        x: hex8(line, "X:"),
        y: hex8(line, "Y:"),
        p: hex8(line, "P:"),
        sp: hex8(line, "SP:"),
        cyc,
    }
}

#[test]
fn nestest_golden_log() {
    let rom = match std::fs::read("tests/data/nestest.nes") {
        Ok(r) => r,
        Err(_) => {
            eprintln!("nestest.nes not present; skipping");
            return;
        }
    };
    let log = std::fs::read_to_string("tests/data/nestest.log").expect("nestest.log");

    let mut nes = Nes::new(&rom).unwrap();
    // automation mode: start at $C000 instead of the reset vector
    nes.cpu.pc = 0xC000;
    nes.cpu.sp = 0xFD;
    nes.cpu.p = 0x24;
    nes.cpu.cycles = 7;

    for (i, line) in log.lines().take(5259).enumerate() {
        let exp = parse_line(line);
        let lineno = i + 1;
        assert_eq!(nes.cpu.pc, exp.pc, "line {lineno}: PC (expected {:04X}, got {:04X})\n{line}", exp.pc, nes.cpu.pc);
        assert_eq!(nes.cpu.a, exp.a, "line {lineno}: A\n{line}");
        assert_eq!(nes.cpu.x, exp.x, "line {lineno}: X\n{line}");
        assert_eq!(nes.cpu.y, exp.y, "line {lineno}: Y\n{line}");
        assert_eq!(nes.cpu.p, exp.p, "line {lineno}: P (expected {:02X}, got {:02X})\n{line}", exp.p, nes.cpu.p);
        assert_eq!(nes.cpu.sp, exp.sp, "line {lineno}: SP\n{line}");
        assert_eq!(nes.cpu.cycles, exp.cyc, "line {lineno}: CYC\n{line}");
        nes.cpu.step();
    }

    // official-opcode test result code at $0002 must be 0 (no failure)
    assert_eq!(nes.cpu.bus.read(0x0002), 0, "nestest reported official-opcode failure");
}
