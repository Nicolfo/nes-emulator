//! Headless AccuracyCoin runner: boots the ROM, presses Start at the main
//! menu (which runs every test), then dumps the pass/fail table at $0400.
//!
//! Usage: cargo run --example accuracy_rom [--release]

use nes_emulator::controller::BTN_START;
use nes_emulator::nes::Nes;

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

fn press(nes: &mut Nes, btn: u8) {
    nes.cpu.bus.controller1.set_button(btn, true);
    for _ in 0..2 {
        nes.run_frame();
    }
    nes.cpu.bus.controller1.set_button(btn, false);
    for _ in 0..2 {
        nes.run_frame();
    }
}

/// Run one test: navigate to `page` (Right presses) and test row, press A,
/// then watch the result address. Prints ErrorCode ($10) transitions.
fn run_single(mut nes: Nes, page: usize, test: usize, result_addr: u16) {
    use nes_emulator::controller::{BTN_A, BTN_DOWN, BTN_RIGHT};
    while ram(&nes, 0x14) as usize != page {
        press(&mut nes, BTN_RIGHT);
    }
    while ram(&nes, 0x16) as usize != test {
        press(&mut nes, BTN_DOWN);
    }
    println!(
        "nav state: tab={} cursor={} height={}",
        ram(&nes, 0x14),
        ram(&nes, 0x16),
        ram(&nes, 0x17)
    );
    nes.cpu.bus.controller1.set_button(BTN_A, true);
    for _ in 0..2 {
        nes.run_frame();
    }
    nes.cpu.bus.controller1.set_button(BTN_A, false);
    let mut last_ec = 0xFFu8;
    for frame in 0..3600 {
        nes.run_frame();
        let ec = ram(&nes, 0x10);
        if ec != last_ec {
            println!("frame {:5}: ErrorCode = {}", frame, ec);
            last_ec = ec;
        }
        let v = ram(&nes, result_addr);
        if v != 0 && v != 3 {
            println!("result = {:#04x} (code {})", v, v >> 2);
            // DUMP_RANGE=hexaddr:len dumps RAM after the test finishes.
            if let Ok(spec) = std::env::var("DUMP_RANGE")
                && let Some((a, l)) = spec.split_once(':')
            {
                let a = u16::from_str_radix(a, 16).unwrap();
                let l: u16 = l.parse().unwrap();
                print!("${:04X}:", a);
                for i in 0..l {
                    print!(" {:02x}", ram(&nes, a + i));
                }
                println!();
            }
            return;
        }
    }
    println!("TIMEOUT, ErrorCode = {}", last_ec);
    for _ in 0..30 {
        nes.cpu.step();
        print!("{:04X} ", nes.cpu.pc);
    }
    println!();
    println!(
        "ppu: scanline={} dot={} status={:02x}",
        nes.cpu.bus.ppu.scanline, nes.cpu.bus.ppu.dot, nes.cpu.bus.ppu.status
    );
}

fn main() {
    let rom = std::fs::read("AccuracyCoin.nes").expect("AccuracyCoin.nes in repo root");
    let mut nes = Nes::new(&rom).unwrap();

    // Let the menu load. $EC is the ROM's debug progress counter; 0x0A = menu live.
    for _ in 0..120 {
        nes.run_frame();
    }
    println!("menu progress $EC = {:#04x}", ram(&nes, 0xEC));

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--watch-sprite0" {
        use nes_emulator::controller::BTN_START;
        nes.cpu.bus.controller1.set_button(BTN_START, true);
        for _ in 0..3 {
            nes.run_frame();
        }
        nes.cpu.bus.controller1.set_button(BTN_START, false);
        // In run-all there is no in-progress marker; key off the previous
        // test's result ($459, sprite overflow) landing.
        while ram(&nes, 0x459) == 0 {
            nes.cpu.step();
        }
        println!(
            "sprite0 test started (overflow result={:#04x})",
            ram(&nes, 0x459)
        );
        let mut frames = 0;
        loop {
            let v = ram(&nes, 0x457);
            if v != 0 && v != 3 {
                println!("result {:#04x}", v);
                break;
            }
            nes.run_frame();
            frames += 1;
            if frames % 5 == 0 || frames < 15 {
                let p = &nes.cpu.bus.ppu;
                println!(
                    "f{:3}: mask={:02x} ctrl={:02x} status={:02x} oam0={:02x?} vram[0..3]={:02x?} ec={}",
                    frames,
                    p.mask,
                    p.ctrl,
                    p.status,
                    &p.oam[0..4],
                    &p.vram[0..3],
                    ram(&nes, 0x10)
                );
            }
            if frames > 600 {
                break;
            }
        }
        return;
    }
    if args.len() > 2 && args[1] == "--markskip" {
        // Mark individual tests skipped ("page:row,page:row"), then run-all.
        use nes_emulator::controller::{BTN_B, BTN_DOWN, BTN_RIGHT, BTN_UP};
        for spec in args[2].split(',') {
            let (p, r) = spec.split_once(':').unwrap();
            let (p, r): (usize, usize) = (p.parse().unwrap(), r.parse().unwrap());
            while ram(&nes, 0x14) as usize != p {
                press(&mut nes, BTN_RIGHT);
            }
            while (ram(&nes, 0x16) as i8) >= 0 {
                press(&mut nes, BTN_UP);
            }
            while ram(&nes, 0x16) as usize != r {
                press(&mut nes, BTN_DOWN);
            }
            press(&mut nes, BTN_B);
            while (ram(&nes, 0x16) as i8) >= 0 {
                press(&mut nes, BTN_UP);
            }
        }
        while ram(&nes, 0x14) != 0 {
            press(&mut nes, BTN_RIGHT);
        }
    } else if args.len() > 1 && args[1] == "--skip" {
        // Mark whole pages as skipped (B at page top), then fall through to run-all.
        use nes_emulator::controller::{BTN_B, BTN_RIGHT};
        for page in args[2].split(',').map(|s| s.parse::<usize>().unwrap()) {
            while ram(&nes, 0x14) != page as u8 {
                press(&mut nes, BTN_RIGHT);
            }
            press(&mut nes, BTN_B);
        }
        while ram(&nes, 0x14) != 0 {
            press(&mut nes, BTN_RIGHT);
        }
    } else if args.len() == 4 && args[1] != "--suite" {
        let page: usize = args[1].parse().unwrap();
        let test: usize = args[2].parse().unwrap();
        let result_addr = u16::from_str_radix(args[3].trim_start_matches("0x"), 16).unwrap();
        println!(
            "pretests: vbl-sync $3A={:#04x} dma-sync $12={:#04x}",
            ram(&nes, 0x3A),
            ram(&nes, 0x12)
        );
        run_single(nes, page, test, result_addr);
        return;
    } else if args.len() >= 3 && args[1] == "--suite" {
        // Run a whole page with A on the page top, then dump $0400-$0491.
        use nes_emulator::controller::{BTN_A, BTN_RIGHT};
        let page: usize = args[2].parse().unwrap();
        while ram(&nes, 0x14) as usize != page {
            press(&mut nes, BTN_RIGHT);
        }
        press(&mut nes, BTN_A);
        // Trace the first test on the page; dump PPU state when its result lands.
        let result_addr: u16 = args.get(3).map_or(0x459, |s| {
            u16::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
        });
        let mut sprite0_seen = false;
        loop {
            let v = ram(&nes, result_addr);
            if v != 0 && v != 3 {
                println!("result = {:#04x}", v);
                break;
            }
            nes.cpu.step();
            if nes.cpu.bus.ppu.status & 0x40 != 0 {
                sprite0_seen = true;
            }
        }
        println!("sprite0 hit ever set: {}", sprite0_seen);
        println!(
            "mask={:02x} ctrl={:02x} oam0={:02x?}",
            nes.cpu.bus.ppu.mask,
            nes.cpu.bus.ppu.ctrl,
            &nes.cpu.bus.ppu.oam[0..4]
        );
        println!("vram[0..8]={:02x?}", &nes.cpu.bus.ppu.vram[0..8]);
        for &(addr, name) in TESTS {
            let v = ram(&nes, addr);
            if v != 0 {
                println!("{:#06x} = {:#04x}  {}", addr, v, name);
            }
        }
        return;
    }

    // Press Start (run every test in the ROM), then release.
    nes.cpu.bus.controller1.set_button(BTN_START, true);
    for _ in 0..3 {
        nes.run_frame();
    }
    nes.cpu.bus.controller1.set_button(BTN_START, false);

    // RunningAllTests = $35: set to 1 while the suite runs, cleared after.
    let mut started = false;
    let mut last_tally = 0u8;
    let mut max_tally = 0u8;
    let mut idle_frames = 0u32;
    for frame in 0..60 * 240 {
        nes.run_frame();
        let tally = ram(&nes, 0x37);
        if tally != last_tally {
            if std::env::var("TALLY_LOG").is_ok() {
                println!("tally {} -> {} at frame {}", last_tally, tally, frame);
            }
            last_tally = tally;
        }
        max_tally = max_tally.max(tally);
        let running = ram(&nes, 0x35);
        if running == 1 {
            started = true;
            idle_frames = 0;
        } else if started {
            // Some tests (e.g. Implied Dummy Reads) zero the whole zero page
            // - including $35/$37 - for ~25 frames mid-test and restore it
            // afterwards, so only treat a clear flag as the end of the run
            // once it has stayed clear well past that.
            idle_frames += 1;
            if idle_frames >= 60 {
                if tally < max_tally {
                    println!(
                        "TALLY RESET {} -> {} (likely console reset)",
                        max_tally, tally
                    );
                }
                println!("all tests finished after {} frames, tally={}", frame, tally);
                break;
            }
        }
    }
    if !started {
        println!("WARNING: run-all never started (Start press not registered?)");
    }

    let mut pass = 0;
    let mut fail = 0;
    let mut not_run = 0;
    for &(addr, name) in TESTS {
        let v = ram(&nes, addr);
        if v == 0 {
            not_run += 1;
            println!("NOT RUN        {}", name);
        } else if v & 1 == 1 {
            pass += 1;
            if v != 1 {
                println!("PASS (variant {:#04x}) {}", v >> 2, name);
            }
        } else {
            fail += 1;
            println!("FAIL (code {:#3}) {}", v >> 2, name);
        }
    }
    println!("\n{} passed, {} failed, {} not run", pass, fail, not_run);
}
