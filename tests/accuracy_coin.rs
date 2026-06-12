use nes_emulator::nes::Nes;
use nes_emulator::cpu::{D, B, U};

fn make_nes(prg: &[u8]) -> Nes {
    let mut data = vec![0u8; 16 + 32768 + 8192];
    data[0..4].copy_from_slice(b"NES\x1A");
    data[4] = 2; // 32KB PRG
    data[5] = 1; // 8KB CHR
    data[16..16 + prg.len()].copy_from_slice(prg);
    // Reset vector at $FFFC-$FFFD (offset 16 + 32768 - 4) points to $8000
    data[16 + 32768 - 4] = 0x00;
    data[16 + 32768 - 3] = 0x80;
    
    Nes::new(&data).unwrap()
}

// ==========================================
// PAGE 1: CPU BEHAVIOR
// ==========================================

#[test]
fn test_rom_is_not_writable() {
    let mut nes = make_nes(&[0x00]); // BRK
    // Try writing to PRG ROM at $8000
    let original = nes.cpu.bus.read(0x8000);
    nes.cpu.bus.write(0x8000, original.wrapping_add(1));
    assert_eq!(nes.cpu.bus.read(0x8000), original, "ROM should not be writable");
}

#[test]
fn test_ram_mirroring() {
    let mut nes = make_nes(&[0x00]);
    // 2KB RAM at $0000-$07FF is mirrored at $0800-$0FFF, $1000-$17FF, $1800-$1FFF
    nes.cpu.bus.write(0x0010, 0x42);
    assert_eq!(nes.cpu.bus.read(0x0810), 0x42);
    assert_eq!(nes.cpu.bus.read(0x1010), 0x42);
    assert_eq!(nes.cpu.bus.read(0x1810), 0x42);

    nes.cpu.bus.write(0x0F00, 0x99); // $0F00 mirrors to $0700
    assert_eq!(nes.cpu.bus.read(0x0700), 0x99);
}

#[test]
fn test_pc_wraparound() {
    // We construct a special ROM where the last byte of memory ($FFFF) is an instruction.
    // Specifically, LDX #$37 (0xA2 0x37).
    // Opcode 0xA2 is at $FFFF, and operand 0x37 wraps to $0000 in RAM.
    let mut data = vec![0u8; 16 + 32768 + 8192];
    data[0..4].copy_from_slice(b"NES\x1A");
    data[4] = 2; // 32KB PRG
    data[5] = 1; // 8KB CHR
    
    // LDX Imm opcode 0xA2 at $FFFF (offset 16 + 32767)
    data[16 + 32767] = 0xA2;
    // Set reset vector to $FFFF
    data[16 + 32768 - 4] = 0xFF;
    data[16 + 32768 - 3] = 0xFF;
    
    let mut nes = Nes::new(&data).unwrap();
    // Put operand at $0000 in RAM
    nes.cpu.bus.write(0x0000, 0x37);
    
    nes.cpu.step();
    assert_eq!(nes.cpu.x, 0x37);
    assert_eq!(nes.cpu.pc, 0x0001); // PC should wrap from $FFFF to $0000, then fetch operand and be $0001
}

#[test]
fn test_decimal_flag() {
    // 1. D flag should not affect ADC or SBC
    let mut nes = make_nes(&[
        0xF8,       // SED (Set decimal flag)
        0x18,       // CLC
        0xA9, 0x09, // LDA #$09
        0x69, 0x01, // ADC #$01
        0x00,       // BRK
    ]);
    for _ in 0..4 {
        nes.cpu.step();
    }
    assert_eq!(nes.cpu.a, 0x0A, "Decimal ADC should still behave as binary (0x0A, not 0x10)");
    assert_eq!(nes.cpu.p & D, D, "D flag should be set");

    // 2. D flag still gets pushed by PHP
    let mut nes = make_nes(&[
        0xF8, // SED
        0x08, // PHP
        0x68, // PLA
        0x00, // BRK
    ]);
    for _ in 0..3 {
        nes.cpu.step();
    }
    assert_eq!(nes.cpu.a & D, D, "D flag should be pushed by PHP");
}

#[test]
fn test_b_flag() {
    // 1. B flag is set by PHP
    let mut nes = make_nes(&[
        0x08, // PHP
        0x68, // PLA
        0x00, // BRK
    ]);
    nes.cpu.step(); // PHP
    nes.cpu.step(); // PLA
    assert_eq!(nes.cpu.a & B, B, "PHP should set B flag in pushed status");
    assert_eq!(nes.cpu.a & U, U, "PHP should set U flag (bit 5) in pushed status");

    // 2. B flag is set by BRK
    // Set interrupt vector at $FFFE-$FFFF to $8010
    let mut data = vec![0u8; 16 + 32768 + 8192];
    data[0..4].copy_from_slice(b"NES\x1A");
    data[4] = 2; // 32KB PRG
    data[5] = 1; // 8KB CHR
    data[16] = 0x00; // BRK at $8000
    // Vector at $FFFE-$FFFF points to $8005
    data[16 + 32768 - 2] = 0x05;
    data[16 + 32768 - 1] = 0x80;
    // Instruction at $8005: PLA
    data[16 + 5] = 0x68;
    
    let mut nes = Nes::new(&data).unwrap();
    nes.cpu.step(); // BRK
    nes.cpu.step(); // PLA
    assert_eq!(nes.cpu.a & B, B, "BRK should set B flag in pushed status");
    assert_eq!(nes.cpu.a & U, U, "BRK should set U flag (bit 5) in pushed status");
}

#[test]
fn test_dummy_read_cycles_lda_page_cross() {
    // 1. A mirror of PPU_STATUS ($2002) should be read twice by LDA $20F2, X (where X = $10).
    let mut nes = make_nes(&[
        0xA2, 0x10,       // LDX #$10
        0xBD, 0xF2, 0x20, // LDA $20F2, X
        0x00,             // BRK
    ]);
    
    // Set scanline to 100 to prevent VBlank from being auto-cleared during prerender cycles
    nes.cpu.bus.ppu.scanline = 100;
    nes.cpu.bus.ppu.status |= 0x80;
    
    nes.cpu.step(); // LDX #$10
    nes.cpu.step(); // LDA $20F2, X
    
    assert_eq!(nes.cpu.a & 0x80, 0, "Dummy read should have cleared VBlank before final read");
}

#[test]
fn test_dummy_read_cycles_lda_no_page_cross() {
    let mut nes = make_nes(&[
        0xA2, 0x08,       // LDX #$08
        0xBD, 0xF2, 0x20, // LDA $20F2, X  (effective address $20FA, mirrors $2002, no page cross)
        0x00,             // BRK
    ]);
    
    nes.cpu.bus.ppu.scanline = 100;
    nes.cpu.bus.ppu.status |= 0x80;
    
    nes.cpu.step(); // LDX
    nes.cpu.step(); // LDA
    
    assert_eq!(nes.cpu.a & 0x80, 0x80, "No page cross -> no dummy read -> final read should see VBlank");
}

#[test]
fn test_dummy_read_cycles_sta_always() {
    let mut nes = make_nes(&[
        0xA9, 0x00,       // LDA #$00
        0xA2, 0x10,       // LDX #$10
        0x9D, 0xF2, 0x20, // STA $20F2, X  (effective address $2102, mirrors $2002, uncorrected $2002, page cross)
        0x00,             // BRK
    ]);
    
    nes.cpu.bus.ppu.scanline = 100;
    nes.cpu.bus.ppu.status |= 0x80;
    
    nes.cpu.step(); // LDA
    nes.cpu.step(); // LDX
    nes.cpu.step(); // STA
    
    let status = nes.cpu.bus.read(0x2002);
    assert_eq!(status & 0x80, 0, "STA dummy read should have cleared VBlank");
}

#[test]
fn test_open_bus_lda_absolute() {
    let mut nes = make_nes(&[
        0xAD, 0xAA, 0x55, // LDA $55AA
        0x00,             // BRK
    ]);
    
    nes.cpu.step();
    assert_eq!(nes.cpu.a, 0x55, "LDA from unmapped $55AA should return high byte of operand (0x55)");
}

#[test]
fn test_open_bus_controller_bits() {
    let mut nes = make_nes(&[0x00]);
    nes.cpu.bus.open_bus = 0xE0;
    let val = nes.cpu.bus.read(0x4016);
    assert_eq!(val & 0xE0, 0xE0, "Upper 3 bits of controller read should be open bus");
}

#[test]
fn test_open_bus_4015_read_bit5() {
    // Bit 5 of a $4015 read comes from the CPU's internal data bus, which a
    // DMC DMA fetch does not update (unlike the external open bus).
    let mut nes = make_nes(&[0x00]);
    nes.cpu.bus.internal_bus = 0x20;
    let val = nes.cpu.bus.read(0x4015);
    assert_eq!(val & 0x20, 0x20, "Bit 5 of $4015 read should be internal bus bit 5");
}

// ==========================================
// PAGE 2: ADDRESSING MODE WRAPAROUND
// ==========================================

#[test]
fn test_abs_indexed_wraparound() {
    let mut nes = make_nes(&[
        0xA2, 0x02,       // LDX #$02
        0xBD, 0xFF, 0xFF, // LDA $FFFF, X
        0x00,
    ]);
    nes.cpu.bus.write(0x0001, 0xE7);
    nes.cpu.step(); // LDX
    nes.cpu.step(); // LDA
    assert_eq!(nes.cpu.a, 0xE7, "Absolute indexed past $FFFF should wrap to Zero Page");
}

#[test]
fn test_zp_indexed_wraparound() {
    let mut nes = make_nes(&[
        0xA2, 0x05,       // LDX #$05
        0xB5, 0xFC,       // LDA $FC, X
        0x00,
    ]);
    nes.cpu.bus.write(0x0001, 0x88);
    nes.cpu.step(); // LDX
    nes.cpu.step(); // LDA
    assert_eq!(nes.cpu.a, 0x88, "ZP indexed past $FF should wrap within ZP ($0000-$00FF)");
}

#[test]
fn test_jmp_indirect_page_wrap_bug() {
    let mut data = vec![0u8; 16 + 32768 + 8192];
    data[0..4].copy_from_slice(b"NES\x1A");
    data[4] = 2;
    data[5] = 1;
    
    data[16] = 0x6C; // Opcode JMP (indirect)
    data[16 + 1] = 0xFF; // Pointer low byte
    data[16 + 2] = 0x80; // Pointer high byte ($80FF)
    
    data[16 + 0xFF] = 0x34; // Low byte of target from $80FF
    // High byte of target is read from $8000 (which contains 0x6C, the JMP opcode)
    
    data[16 + 32768 - 4] = 0x00;
    data[16 + 32768 - 3] = 0x80;
    
    let mut nes = Nes::new(&data).unwrap();
    nes.cpu.step();
    assert_eq!(nes.cpu.pc, 0x6C34, "JMP indirect at page boundary should wrap to page start for high byte");
}

// ==========================================
// PAGES 3-11: UNOFFICIAL INSTRUCTIONS
// ==========================================

#[test]
fn test_unofficial_sbc_eb() {
    let mut nes = make_nes(&[
        0x38,       // SEC
        0xA9, 0x05, // LDA #$05
        0xEB, 0x02, // SBC #$02 (unofficial)
        0x00,
    ]);
    nes.cpu.step(); // SEC
    nes.cpu.step(); // LDA
    nes.cpu.step(); // SBC
    assert_eq!(nes.cpu.a, 3);
}

#[test]
fn test_unofficial_sax() {
    let mut nes = make_nes(&[
        0xA9, 0x0F, // LDA #$0F
        0xA2, 0xF0, // LDX #$F0
        0x87, 0x20, // SAX $20 (unofficial ZP)
        0x00,
    ]);
    nes.cpu.step(); // LDA
    nes.cpu.step(); // X
    nes.cpu.step(); // SAX
    assert_eq!(nes.cpu.bus.read(0x0020), 0x00, "SAX should write A & X (0x0F & 0xF0 = 0x00)");
}
