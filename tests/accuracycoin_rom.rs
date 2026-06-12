//! Runs the full AccuracyCoin test ROM (https://github.com/100thCoin/AccuracyCoin)
//! and asserts every test passes. Skips itself when the ROM is absent; see
//! docs/accuracy.md for how to obtain it. Run with `--release` — the suite
//! emulates ~75 seconds of NES time.

use nes_emulator::controller::BTN_START;
use nes_emulator::nes::Nes;

/// Result-table addresses ($0400 page) in menu order, with test names.
const TESTS: &[(u16, &str)] = &[
    (0x0405, "ROM is not writable"),
    (0x0403, "RAM Mirroring"),
    (0x044D, "PC Wraparound"),
    (0x0474, "The Decimal Flag"),
    (0x0475, "The B Flag"),
    (0x0406, "Dummy read cycles"),
    (0x0407, "Dummy write cycles"),
    (0x0408, "Open Bus"),
    (0x047D, "All NOP instructions"),
    (0x046E, "Absolute Indexed"),
    (0x046F, "Zero Page Indexed"),
    (0x0470, "Indirect"),
    (0x0471, "Indirect, X"),
    (0x0472, "Indirect, Y"),
    (0x0473, "Relative"),
    (0x0409, "$03   SLO indirect,X"),
    (0x040A, "$07   SLO zeropage"),
    (0x040B, "$0F   SLO absolute"),
    (0x040C, "$13   SLO indirect,Y"),
    (0x040D, "$17   SLO zeropage,X"),
    (0x040E, "$1B   SLO absolute,Y"),
    (0x040F, "$1F   SLO absolute,X"),
    (0x0419, "$23   RLA indirect,X"),
    (0x041A, "$27   RLA zeropage"),
    (0x041B, "$2F   RLA absolute"),
    (0x041C, "$33   RLA indirect,Y"),
    (0x041D, "$37   RLA zeropage,X"),
    (0x041E, "$3B   RLA absolute,Y"),
    (0x041F, "$3F   RLA absolute,X"),
    (0x0420, "$43   SRE indirect,X"),
    (0x047F, "$47   SRE zeropage"),
    (0x0422, "$4F   SRE absolute"),
    (0x0423, "$53   SRE indirect,Y"),
    (0x0424, "$57   SRE zeropage,X"),
    (0x0425, "$5B   SRE absolute,Y"),
    (0x0426, "$5F   SRE absolute,X"),
    (0x0427, "$63   RRA indirect,X"),
    (0x0428, "$67   RRA zeropage"),
    (0x0429, "$6F   RRA absolute"),
    (0x042A, "$73   RRA indirect,Y"),
    (0x042B, "$77   RRA zeropage,X"),
    (0x042C, "$7B   RRA absolute,Y"),
    (0x042D, "$7F   RRA absolute,X"),
    (0x042E, "$83   SAX indirect,X"),
    (0x042F, "$87   SAX zeropage"),
    (0x0430, "$8F   SAX absolute"),
    (0x0431, "$97   SAX zeropage,Y"),
    (0x0432, "$A3   LAX indirect,X"),
    (0x0433, "$A7   LAX zeropage"),
    (0x0434, "$AF   LAX absolute"),
    (0x0435, "$B3   LAX indirect,Y"),
    (0x0436, "$B7   LAX zeropage,Y"),
    (0x0437, "$BF   LAX absolute,Y"),
    (0x0438, "$C3   DCP indirect,X"),
    (0x0439, "$C7   DCP zeropage"),
    (0x043A, "$CF   DCP absolute"),
    (0x043B, "$D3   DCP indirect,Y"),
    (0x043C, "$D7   DCP zeropage,X"),
    (0x043D, "$DB   DCP absolute,Y"),
    (0x043E, "$DF   DCP absolute,X"),
    (0x043F, "$E3   ISC indirect,X"),
    (0x0440, "$E7   ISC zeropage"),
    (0x0441, "$EF   ISC absolute"),
    (0x0442, "$F3   ISC indirect,Y"),
    (0x0443, "$F7   ISC zeropage,X"),
    (0x0444, "$FB   ISC absolute,Y"),
    (0x0445, "$FF   ISC absolute,X"),
    (0x0446, "$93   SHA indirect,Y"),
    (0x0447, "$9F   SHA absolute,Y"),
    (0x0448, "$9B   SHS absolute,Y"),
    (0x0449, "$9C   SHY absolute,X"),
    (0x044A, "$9E   SHX absolute,Y"),
    (0x044B, "$BB   LAE absolute,Y"),
    (0x0410, "$0B   ANC Immediate"),
    (0x0411, "$2B   ANC Immediate"),
    (0x0412, "$4B   ASR Immediate"),
    (0x0413, "$6B   ARR Immediate"),
    (0x0414, "$8B   ANE Immediate"),
    (0x0415, "$AB   LXA Immediate"),
    (0x0416, "$CB   AXS Immediate"),
    (0x0417, "$EB   SBC Immediate"),
    (0x0461, "Interrupt flag latency"),
    (0x0462, "NMI Overlap BRK"),
    (0x0463, "NMI Overlap IRQ"),
    (0x046C, "DMA + Open Bus"),
    (0x0488, "DMA + $2002 Read"),
    (0x044C, "DMA + $2007 Read"),
    (0x044F, "DMA + $2007 Write"),
    (0x045D, "DMA + $4015 Read"),
    (0x045E, "DMA + $4016 Read"),
    (0x046B, "DMC DMA Bus Conflicts"),
    (0x0477, "DMC DMA + OAM DMA"),
    (0x0479, "Explicit DMA Abort"),
    (0x0478, "Implicit DMA Abort"),
    (0x0465, "Length Counter"),
    (0x0466, "Length Table"),
    (0x0467, "Frame Counter IRQ"),
    (0x0468, "Frame Counter 4-step"),
    (0x0469, "Frame Counter 5-step"),
    (0x046A, "Delta Modulation Channel"),
    (0x045C, "APU Register Activation"),
    (0x045F, "Controller Strobing"),
    (0x047A, "Controller Clocking"),
    (0x0485, "CHR ROM is not writable"),
    (0x0404, "PPU Register Mirroring"),
    (0x044E, "PPU Register Open Bus"),
    (0x0476, "PPU Read Buffer"),
    (0x047E, "Palette RAM Quirks"),
    (0x0486, "Rendering Flag Behavior"),
    (0x048A, "$2007 read w/ rendering"),
    (0x0450, "VBlank beginning"),
    (0x0451, "VBlank end"),
    (0x0452, "NMI Control"),
    (0x0453, "NMI Timing"),
    (0x0454, "NMI Suppression"),
    (0x0455, "NMI at VBlank end"),
    (0x0456, "NMI disabled at VBlank"),
    (0x0459, "Sprite overflow behavior"),
    (0x0457, "Sprite 0 Hit behavior"),
    (0x048D, "$2002 flag timing"),
    (0x0489, "Suddenly Resize Sprite"),
    (0x0458, "Arbitrary Sprite zero"),
    (0x045A, "Misaligned OAM behavior"),
    (0x045B, "Address $2004 behavior"),
    (0x047B, "OAM Corruption"),
    (0x0480, "INC $4014"),
    (0x0481, "Attributes As Tiles"),
    (0x0482, "t Register Quirks"),
    (0x0483, "Stale BG Shift Registers"),
    (0x048F, "Stale Sprite Shift Regs"),
    (0x0487, "BG Serial In"),
    (0x0484, "Sprites On Scanline 0"),
    (0x048C, "$2004 Stress Test"),
    (0x048E, "$2007 Stress Test"),
    (0x0491, "ALE + Read"),
    (0x0460, "Instruction Timing"),
    (0x046D, "Implied Dummy Reads"),
    (0x048B, "Branch Dummy Reads"),
    (0x047C, "JSR Edge Cases"),
    (0x0490, "Internal Data Bus"),
];

fn ram(nes: &Nes, addr: u16) -> u8 {
    nes.cpu.bus.ram[(addr & 0x07FF) as usize]
}

#[test]
fn accuracycoin_full_suite() {
    let Ok(rom) = std::fs::read("AccuracyCoin.nes") else {
        eprintln!("AccuracyCoin.nes not found in project root - skipping (see docs/accuracy.md)");
        return;
    };
    let mut nes = Nes::new(&rom).unwrap();

    // Let the menu load ($EC reaches 0x0A when live), then press Start to run
    // every test.
    for _ in 0..120 {
        nes.run_frame();
    }
    assert_eq!(ram(&nes, 0xEC), 0x0A, "menu did not finish loading");
    nes.cpu.bus.controller1.set_button(BTN_START, true);
    for _ in 0..3 {
        nes.run_frame();
    }
    nes.cpu.bus.controller1.set_button(BTN_START, false);

    // RunningAllTests ($35) is 1 while the suite runs. Some tests zero the
    // whole zero page for ~25 frames mid-run, so only treat a clear flag as
    // completion once it has stayed clear for a full second.
    let mut started = false;
    let mut idle_frames = 0u32;
    let mut finished = false;
    for _ in 0..60 * 240 {
        nes.run_frame();
        if ram(&nes, 0x35) == 1 {
            started = true;
            idle_frames = 0;
        } else if started {
            idle_frames += 1;
            if idle_frames >= 60 {
                finished = true;
                break;
            }
        }
    }
    assert!(started && finished, "run-all did not start/finish");

    let mut failures = Vec::new();
    for &(addr, name) in TESTS {
        let v = ram(&nes, addr);
        if v == 0 {
            failures.push(format!("NOT RUN          {name}"));
        } else if v & 1 == 0 {
            failures.push(format!("FAIL (code {:3}) {name}", v >> 2));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} AccuracyCoin tests failed:\n{}",
        failures.len(),
        TESTS.len(),
        failures.join("\n")
    );
}
