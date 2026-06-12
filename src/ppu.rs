use crate::mapper::{Mapper, Mirroring};
use crate::palette::NES_PALETTE;

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 240;

#[derive(Clone, Copy, Default)]
struct SpriteRow {
    /// X-position down-counter: decrements every visible dot regardless of
    /// rendering enable; the shifter outputs once it reaches 0.
    counter: u8,
    /// Counter mode. Counters are armed ("counting") on dot 339 only if
    /// rendering is enabled there; once they hit 0 they go "halted" (drawing)
    /// and stay halted until re-armed.
    counting: bool,
    /// Pattern shifters: shift (MSB out) only while rendering is enabled.
    pat_lo: u8,
    pat_hi: u8,
    attr: u8,
    is_zero: bool,
}

pub struct Ppu {
    // loopy registers
    v: u16,
    t: u16,
    fine_x: u8,
    w: bool,

    pub ctrl: u8,
    pub mask: u8,
    pub status: u8,
    // PPU I/O data bus latch: holds the last value driven on the $200x bus.
    // Each bit decays to 0 independently if not refreshed (analog behavior).
    io_bus: u8,
    io_bus_ts: [u64; 8],
    dots: u64,

    oam_addr: u8,
    pub oam: [u8; 256],
    sprites: [SpriteRow; 8],
    sprite_count: usize,

    // ---- dot-accurate sprite pipeline ----
    secondary_oam: [u8; 32],
    /// Secondary OAM address as the hardware tracks it per dot (0-31).
    sec_addr: u8,
    /// Value visible on $2004 reads while rendering.
    oam_bus: u8,
    /// Primary OAM byte pointer during evaluation (byte-granular: a
    /// misaligned OAMADDR at dot 65 misaligns every sprite).
    eval_ptr: u8,
    eval_latch: u8,
    /// Bytes left to copy for the current in-range sprite (0 = comparing Y).
    eval_copying: u8,
    /// Candidates examined this line (eval ends after 64).
    eval_candidates: u8,
    eval_done: bool,
    sec_full: bool,
    /// 3 dummy reads left after the overflow flag was set.
    overflow_dummy: u8,
    /// The first candidate evaluated this line was in range: the sprite in
    /// secondary OAM slot 0 acts as sprite zero on the next line.
    sprite0_next: bool,
    sprite0_cur: bool,
    /// OAM corruption row seeded by disabling rendering mid-line; applied on
    /// the next rendered dot of a sprite-evaluation line.
    pending_corruption: Option<u8>,
    /// $2001 writes take effect ~2 dots later.
    pending_mask: Option<(u8, u8)>,

    // ---- PPU data state machine ($2007 during rendering) ----
    /// Last byte the rendering pipeline fetched from memory.
    last_fetch_val: u8,
    /// Dots until the $2007 state machine's Read fires and refills the read
    /// buffer from the rendering bus (0 = idle).
    capture_delay: u8,
    /// The state machine's Read collides with a pipeline fetch this dot:
    /// the octal latch feeds back, replacing the fetch address low byte
    /// with the previous fetch's data.
    bus_conflict: bool,
    /// Sprite pattern fetch scratch (per 8-dot group).
    spr_pat_addr: u16,
    spr_pat_lo: u8,

    pub vram: [u8; 0x800],
    palette: [u8; 32],
    read_buffer: u8,

    pub scanline: i16, // -1 = pre-render, 0..239 visible, 241..260 vblank
    pub dot: u16,      // 0..340
    odd_frame: bool,
    suppress_vbl: bool,
    pub frame_complete: bool,

    // background pipeline
    bg_pat_lo: u16,
    bg_pat_hi: u16,
    bg_attr_lo: u16,
    bg_attr_hi: u16,
    nt_latch: u8,
    at_latch: u8,
    pat_lo_latch: u8,
    pat_hi_latch: u8,

    pub framebuffer: Vec<u8>, // RGBA, 256*240*4
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppu {
    pub fn new() -> Self {
        Ppu {
            v: 0,
            t: 0,
            fine_x: 0,
            w: false,
            ctrl: 0,
            mask: 0,
            status: 0,
            io_bus: 0,
            io_bus_ts: [0; 8],
            dots: 0,
            oam_addr: 0,
            oam: [0; 256],
            sprites: [SpriteRow::default(); 8],
            sprite_count: 0,
            secondary_oam: [0xFF; 32],
            sec_addr: 0,
            oam_bus: 0xFF,
            eval_ptr: 0,
            eval_latch: 0,
            eval_copying: 0,
            eval_candidates: 0,
            eval_done: false,
            sec_full: false,
            overflow_dummy: 0,
            sprite0_next: false,
            sprite0_cur: false,
            pending_corruption: None,
            pending_mask: None,
            last_fetch_val: 0,
            capture_delay: 0,
            bus_conflict: false,
            spr_pat_addr: 0,
            spr_pat_lo: 0,
            vram: [0; 0x800],
            palette: [0; 32],
            read_buffer: 0,
            scanline: -1,
            dot: 0,
            odd_frame: false,
            suppress_vbl: false,
            frame_complete: false,
            bg_pat_lo: 0,
            bg_pat_hi: 0,
            bg_attr_lo: 0,
            bg_attr_hi: 0,
            nt_latch: 0,
            at_latch: 0,
            pat_lo_latch: 0,
            pat_hi_latch: 0,
            framebuffer: vec![0; WIDTH * HEIGHT * 4],
        }
    }

    /// Level of the NMI output: VBlank flag AND NMI-enable bit.
    pub fn nmi_line(&self) -> bool {
        self.ctrl & 0x80 != 0 && self.status & 0x80 != 0
    }

    fn rendering_enabled(&self) -> bool {
        self.mask & 0x18 != 0
    }

    // ---- I/O bus latch (open bus) ----

    /// Bits decay to 0 if not refreshed for ~25 frames.
    const IO_BUS_DECAY_DOTS: u64 = 25 * 89_342;

    fn io_bus_read(&self) -> u8 {
        let mut v = self.io_bus;
        for bit in 0..8 {
            if self.dots.saturating_sub(self.io_bus_ts[bit]) > Self::IO_BUS_DECAY_DOTS {
                v &= !(1 << bit);
            }
        }
        v
    }

    fn io_bus_refresh(&mut self, mask: u8, val: u8) {
        self.io_bus = (self.io_bus & !mask) | (val & mask);
        for bit in 0..8 {
            if mask & (1 << bit) != 0 {
                self.io_bus_ts[bit] = self.dots;
            }
        }
    }

    // ---- register interface ($2000-$2007) ----

    pub fn read_register(&mut self, reg: u16, cart: &mut dyn Mapper) -> u8 {
        match reg & 7 {
            2 => {
                let mut res = (self.status & 0xE0) | (self.io_bus_read() & 0x1F);
                // The vblank flag is latched when M2 rises but the sprite
                // flags are sampled when M2 falls, ~1.9 PPU dots later: a
                // read landing on pre-render dot 1 (clear tick not yet
                // processed) already sees the sprite flags cleared.
                if self.scanline == -1 && self.dot == 1 {
                    res &= !0x60;
                }
                // Race: reading one PPU clock before VBlank is set returns
                // clear and prevents the flag (and thus the NMI) entirely.
                if self.scanline == 241 && self.dot == 1 {
                    self.suppress_vbl = true;
                }
                self.status &= !0x80; // clear vblank
                self.w = false;
                self.io_bus_refresh(0xE0, res); // only the top 3 bits are driven
                res
            }
            4 => {
                // While rendering, reads expose the OAM bus the sprite
                // pipeline is driving ($FF during the clear phase, the byte
                // under evaluation, the secondary-OAM byte being fetched).
                let res = if self.rendering_enabled() && self.scanline < 240 {
                    self.oam_bus
                } else {
                    let mut v = self.oam[self.oam_addr as usize];
                    if self.oam_addr & 3 == 2 {
                        v &= 0xE3; // attribute bits 2-4 don't exist in OAM
                    }
                    v
                };
                self.io_bus_refresh(0xFF, res);
                res
            }
            7 => {
                // During rendering the buffer refill goes through the PPU
                // data state machine: it lands ~4 dots later from whatever
                // the rendering pipeline drives on the bus.
                if self.rendering_enabled() && self.scanline < 240 {
                    // The v increment (rendering glitch) also waits for the
                    // state machine; fetches before it use the old address.
                    let res = self.read_buffer;
                    self.capture_delay = std::env::var("NES_2007_DELAY")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(5u8);
                    self.io_bus_refresh(0xFF, res);
                    return res;
                }
                let addr = self.v & 0x3FFF;
                let res;
                if addr >= 0x3F00 {
                    // Palette reads: 6 bits from palette RAM, top 2 from the
                    // bus. Greyscale mode forces the low 4 bits to zero.
                    let grey_mask = if self.mask & 1 != 0 { 0x30 } else { 0x3F };
                    res = (self.palette_read(addr) & grey_mask) | (self.io_bus_read() & 0xC0);
                    // buffer gets the nametable byte "underneath" the palette
                    self.read_buffer = self.mem_read(addr & 0x2FFF, cart);
                    self.io_bus_refresh(0x3F, res);
                } else {
                    res = self.read_buffer;
                    self.read_buffer = self.mem_read(addr, cart);
                    self.io_bus_refresh(0xFF, res);
                }
                self.increment_v_after_2007();
                res
            }
            _ => self.io_bus_read(),
        }
    }

    pub fn write_register(&mut self, reg: u16, val: u8, cart: &mut dyn Mapper) {
        self.io_bus_refresh(0xFF, val);
        match reg & 7 {
            0 => {
                self.ctrl = val;
                self.t = (self.t & !0x0C00) | (((val & 3) as u16) << 10);
            }
            1 => {
                // $2001 takes effect a few dots after the write (hardware:
                // 2-5 depending on alignment). Overridable for experiments.
                let d = std::env::var("NES_MASK_DELAY")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(4u8);
                self.pending_mask = Some((val, d));
            }
            3 => self.oam_addr = val,
            4 => {
                if self.rendering_enabled() && self.scanline < 240 {
                    // Writes during rendering don't reach OAM; they bump the
                    // high 6 bits of OAMADDR instead.
                    self.oam_addr = self.oam_addr.wrapping_add(4) & 0xFC;
                } else {
                    self.oam[self.oam_addr as usize] = val;
                    self.oam_addr = self.oam_addr.wrapping_add(1);
                }
            }
            5 => {
                if !self.w {
                    self.t = (self.t & !0x001F) | ((val >> 3) as u16);
                    self.fine_x = val & 7;
                } else {
                    self.t = (self.t & !0x73E0)
                        | (((val & 7) as u16) << 12)
                        | (((val >> 3) as u16) << 5);
                }
                self.w = !self.w;
            }
            6 => {
                if !self.w {
                    self.t = (self.t & 0x00FF) | (((val & 0x3F) as u16) << 8);
                } else {
                    self.t = (self.t & 0xFF00) | val as u16;
                    self.v = self.t;
                }
                self.w = !self.w;
            }
            7 => {
                self.mem_write(self.v & 0x3FFF, val, cart);
                self.increment_v_after_2007();
            }
            _ => {}
        }
    }

    /// $2007 access increments v; during rendering this glitches into a
    /// simultaneous coarse-X and Y increment.
    fn increment_v_after_2007(&mut self) {
        if self.rendering_enabled() && self.scanline < 240 {
            self.increment_coarse_x();
            self.increment_y();
        } else {
            let inc = if self.ctrl & 0x04 != 0 { 32 } else { 1 };
            self.v = self.v.wrapping_add(inc) & 0x7FFF;
        }
    }

    // ---- internal memory ----

    fn mirror_nt(&self, addr: u16, mirroring: Mirroring) -> usize {
        let a = (addr & 0x0FFF) as usize;
        match mirroring {
            Mirroring::Vertical => a & 0x07FF,
            Mirroring::Horizontal => ((a >> 1) & 0x400) | (a & 0x3FF),
        }
    }

    fn mem_read(&mut self, addr: u16, cart: &mut dyn Mapper) -> u8 {
        let addr = addr & 0x3FFF;
        if addr < 0x2000 {
            cart.ppu_read(addr)
        } else if addr < 0x3F00 {
            self.vram[self.mirror_nt(addr, cart.mirroring())]
        } else {
            self.palette_read(addr)
        }
    }

    /// A rendering-pipeline memory fetch: records the byte on the PPU data
    /// bus and applies the octal-latch feedback when the $2007 state
    /// machine's Read collides with this fetch's ALE.
    fn fetch_mem(&mut self, addr: u16, cart: &mut dyn Mapper) -> u8 {
        let addr = if self.bus_conflict {
            (addr & 0xFF00) | self.last_fetch_val as u16
        } else {
            addr
        };
        let v = self.mem_read(addr, cart);
        self.last_fetch_val = v;
        v
    }

    fn mem_write(&mut self, addr: u16, val: u8, cart: &mut dyn Mapper) {
        let addr = addr & 0x3FFF;
        if addr < 0x2000 {
            cart.ppu_write(addr, val);
        } else if addr < 0x3F00 {
            self.vram[self.mirror_nt(addr, cart.mirroring())] = val;
        } else {
            self.palette[Self::palette_index(addr)] = val & 0x3F;
        }
    }

    fn palette_index(addr: u16) -> usize {
        let mut i = (addr & 0x1F) as usize;
        // $3F10/$3F14/$3F18/$3F1C mirror $3F00/$3F04/$3F08/$3F0C
        if i & 0x13 == 0x10 {
            i &= !0x10;
        }
        i
    }

    fn palette_read(&self, addr: u16) -> u8 {
        self.palette[Self::palette_index(addr)]
    }

    // ---- scrolling helpers (loopy) ----

    fn increment_coarse_x(&mut self) {
        if self.v & 0x001F == 31 {
            self.v &= !0x001F;
            self.v ^= 0x0400; // switch horizontal nametable
        } else {
            self.v += 1;
        }
    }

    fn increment_y(&mut self) {
        if self.v & 0x7000 != 0x7000 {
            self.v += 0x1000;
        } else {
            self.v &= !0x7000;
            let mut coarse_y = (self.v >> 5) & 0x1F;
            if coarse_y == 29 {
                coarse_y = 0;
                self.v ^= 0x0800; // switch vertical nametable
            } else if coarse_y == 31 {
                coarse_y = 0;
            } else {
                coarse_y += 1;
            }
            self.v = (self.v & !0x03E0) | (coarse_y << 5);
        }
    }

    fn copy_horizontal(&mut self) {
        self.v = (self.v & !0x041F) | (self.t & 0x041F);
    }

    fn copy_vertical(&mut self) {
        self.v = (self.v & !0x7BE0) | (self.t & 0x7BE0);
    }

    // ---- background pipeline ----

    fn load_shifters(&mut self) {
        self.bg_pat_lo = (self.bg_pat_lo & 0xFF00) | self.pat_lo_latch as u16;
        self.bg_pat_hi = (self.bg_pat_hi & 0xFF00) | self.pat_hi_latch as u16;
        self.bg_attr_lo =
            (self.bg_attr_lo & 0xFF00) | if self.at_latch & 1 != 0 { 0xFF } else { 0 };
        self.bg_attr_hi =
            (self.bg_attr_hi & 0xFF00) | if self.at_latch & 2 != 0 { 0xFF } else { 0 };
    }

    fn shift(&mut self) {
        // Serial inputs: 0 into the low pattern plane, 1 into the high
        // plane; attribute shifters pull from the current attribute latch.
        // Reloads normally overwrite these bits before they reach the
        // output; they only become visible when a reload is skipped
        // (rendering blanked around the load dot).
        self.bg_pat_lo <<= 1;
        self.bg_pat_hi = (self.bg_pat_hi << 1) | 1;
        self.bg_attr_lo = (self.bg_attr_lo << 1) | (self.at_latch as u16 & 1);
        self.bg_attr_hi = (self.bg_attr_hi << 1) | ((self.at_latch as u16 >> 1) & 1);
    }

    fn bg_fetch(&mut self, cart: &mut dyn Mapper) {
        match (self.dot - 1) % 8 {
            0 => {
                self.load_shifters();
                self.nt_latch = self.fetch_mem(0x2000 | (self.v & 0x0FFF), cart);
            }
            2 => {
                let addr =
                    0x23C0 | (self.v & 0x0C00) | ((self.v >> 4) & 0x38) | ((self.v >> 2) & 0x07);
                let mut at = self.fetch_mem(addr, cart);
                if (self.v >> 5) & 2 != 0 {
                    at >>= 4;
                }
                if self.v & 2 != 0 {
                    at >>= 2;
                }
                self.at_latch = at & 3;
            }
            4 => {
                let fine_y = (self.v >> 12) & 7;
                let base = ((self.ctrl as u16 & 0x10) << 8) | ((self.nt_latch as u16) << 4);
                self.pat_lo_latch = self.fetch_mem(base + fine_y, cart);
            }
            6 => {
                let fine_y = (self.v >> 12) & 7;
                let base = ((self.ctrl as u16 & 0x10) << 8) | ((self.nt_latch as u16) << 4);
                self.pat_hi_latch = self.fetch_mem(base + fine_y + 8, cart);
            }
            7 => self.increment_coarse_x(),
            _ => {}
        }
    }

    // ---- sprites ----

    fn sprite_height(&self) -> i16 {
        if self.ctrl & 0x20 != 0 { 16 } else { 8 }
    }

    /// In-range test: the sprite with OAM Y byte `y` covers row `line - y`.
    fn in_range(&self, line: i16, y: u8) -> bool {
        let row = line - y as i16;
        row >= 0 && row < self.sprite_height()
    }

    /// One dot of the sprite pipeline (rendering enabled, line -1..239).
    fn sprite_pipeline_dot(&mut self, cart: &mut dyn Mapper) {
        let d = self.dot;
        let prerender = self.scanline == -1;
        // Dots 1-64 (visible lines): secondary OAM clear, $FF on the bus.
        // The pre-render line neither clears nor evaluates: secondary OAM
        // keeps scanline 239's sprites (the "sprites on scanline 0" quirk).
        if (1..=64).contains(&d) && !prerender {
            self.sec_addr = ((d - 1) / 2) as u8;
            self.oam_bus = 0xFF;
            if d.is_multiple_of(2) {
                self.secondary_oam[self.sec_addr as usize] = 0xFF;
            }
            if d == 64 {
                // Arm evaluation: it starts at the live OAMADDR, byte-granular.
                self.eval_ptr = self.oam_addr;
                self.eval_copying = 0;
                self.eval_candidates = 0;
                self.eval_done = false;
                self.sec_full = false;
                self.overflow_dummy = 0;
                self.sec_addr = 0;
                self.sprite0_next = false;
            }
            return;
        }
        // Dots 65-256 (visible lines): evaluation, 2-dot cadence.
        if (65..=256).contains(&d) && !prerender {
            if d % 2 == 1 {
                let mut v = self.oam[self.eval_ptr as usize];
                if self.eval_ptr & 3 == 2 {
                    v &= 0xE3;
                }
                self.eval_latch = v;
                self.oam_bus = v;
                return;
            }
            if self.eval_done {
                // Writes disabled: the scan keeps walking primary OAM on odd
                // dots while even dots read back secondary OAM.
                self.oam_bus = self.secondary_oam[(self.sec_addr & 31) as usize];
                self.eval_ptr = self.eval_ptr.wrapping_add(4);
                return;
            }
            if self.overflow_dummy > 0 {
                self.oam_bus = self.secondary_oam[(self.sec_addr & 31) as usize];
                self.overflow_dummy -= 1;
                self.eval_ptr = self.eval_ptr.wrapping_add(1);
                if self.overflow_dummy == 0 {
                    // m resets to 0 when the dummy reads finish.
                    self.eval_ptr &= !3;
                    self.eval_done = true;
                }
                return;
            }
            if self.eval_copying > 0 {
                self.secondary_oam[self.sec_addr as usize] = self.eval_latch;
                self.sec_addr += 1;
                self.eval_ptr = self.eval_ptr.wrapping_add(1);
                self.eval_copying -= 1;
                if self.eval_copying == 0 {
                    self.eval_candidates += 1;
                    if self.eval_candidates >= 64 {
                        self.eval_done = true;
                    }
                    if self.sec_addr >= 32 {
                        self.sec_full = true;
                        self.sec_addr = 0;
                    }
                }
                return;
            }
            if !self.sec_full {
                // Comparing a Y byte; it is written to secondary OAM even
                // when out of range.
                self.secondary_oam[self.sec_addr as usize] = self.eval_latch;
                if self.in_range(self.scanline, self.eval_latch) {
                    if self.eval_candidates == 0 {
                        self.sprite0_next = true;
                    }
                    self.eval_copying = 3;
                    self.sec_addr += 1;
                    self.eval_ptr = self.eval_ptr.wrapping_add(1);
                } else {
                    self.eval_ptr = self.eval_ptr.wrapping_add(4);
                    self.eval_candidates += 1;
                    if self.eval_candidates >= 64 {
                        self.eval_done = true;
                    }
                }
            } else {
                // Secondary OAM full: buggy overflow scan; even dots read
                // back secondary OAM (writes are disabled).
                self.oam_bus = self.secondary_oam[(self.sec_addr & 31) as usize];
                if self.in_range(self.scanline, self.eval_latch) {
                    self.status |= 0x20;
                    self.overflow_dummy = 3;
                    self.eval_ptr = self.eval_ptr.wrapping_add(1);
                } else {
                    // The famous diagonal: n and m both increment.
                    let n = (self.eval_ptr >> 2).wrapping_add(1) & 0x3F;
                    let m = (self.eval_ptr.wrapping_add(1)) & 3;
                    self.eval_ptr = (n << 2) | m;
                    self.eval_candidates += 1;
                    if self.eval_candidates >= 64 {
                        self.eval_done = true;
                    }
                }
            }
            return;
        }
        // Dots 257-320 (visible + pre-render): sprite fetches from secondary
        // OAM. The pre-render line does its in-range math with scanline
        // 261 & 255 = 5, picking up stale secondary OAM.
        if (257..=320).contains(&d) {
            if d == 257 {
                self.sprite0_cur = self.sprite0_next;
                self.sprite_count = 8;
            }
            let g = ((d - 257) / 8) as usize;
            let k = (d - 257) % 8;
            let base = g * 4;
            self.sec_addr = match k {
                0 => {
                    self.oam_bus = self.secondary_oam[base];
                    (base + 1) as u8
                }
                1 => {
                    self.oam_bus = self.secondary_oam[base + 1];
                    (base + 2) as u8
                }
                2 => {
                    // Attribute bits were already masked when copied in.
                    self.oam_bus = self.secondary_oam[base + 2];
                    (base + 3) as u8
                }
                _ => {
                    self.oam_bus = self.secondary_oam[base + 3];
                    if k == 7 {
                        (base + 4).min(31) as u8
                    } else {
                        (base + 3) as u8
                    }
                }
            };
            // Bus cadence: two garbage NT reads, then the two pattern planes.
            match k {
                0 | 2 => {
                    self.fetch_mem(0x2000 | (self.v & 0x0FFF), cart);
                }
                4 => {
                    self.spr_pat_addr = self.sprite_pat_addr(g);
                    self.spr_pat_lo = self.fetch_mem(self.spr_pat_addr, cart);
                }
                6 => {
                    let hi = self.fetch_mem(self.spr_pat_addr + 8, cart);
                    self.load_sprite_slot(g, self.spr_pat_lo, hi);
                }
                _ => {}
            }
            return;
        }
        // Dots 337-340: dummy nametable fetches.
        if d == 337 || d == 339 {
            self.fetch_mem(0x2000 | (self.v & 0x0FFF), cart);
        }
        // Dots 321-340 and dot 0: reads return the first secondary OAM byte.
        if d >= 321 {
            self.oam_bus = self.secondary_oam[0];
        }
    }

    /// Pattern address for sprite slot `g` of secondary OAM.
    fn sprite_pat_addr(&self, g: usize) -> u16 {
        let y = self.secondary_oam[g * 4];
        let tile = self.secondary_oam[g * 4 + 1];
        let attr = self.secondary_oam[g * 4 + 2];
        // The pre-render line is treated as scanline 5 (261 & 255) here.
        let line: i16 = if self.scanline == -1 {
            5
        } else {
            self.scanline
        };
        let height = self.sprite_height();
        let mut row = (line - y as i16).rem_euclid(height);
        if attr & 0x80 != 0 {
            row = height - 1 - row; // vertical flip
        }
        if height == 16 {
            let table = ((tile & 1) as u16) << 12;
            let mut t = (tile & 0xFE) as u16;
            if row >= 8 {
                t += 1;
            }
            table | (t << 4) | (row as u16 & 7)
        } else {
            ((self.ctrl as u16 & 0x08) << 9) | ((tile as u16) << 4) | row as u16
        }
    }

    /// Load sprite slot `g` with the fetched pattern bytes.
    fn load_sprite_slot(&mut self, g: usize, mut pat_lo: u8, mut pat_hi: u8) {
        let y = self.secondary_oam[g * 4];
        let attr = self.secondary_oam[g * 4 + 2];
        let x = self.secondary_oam[g * 4 + 3];
        let line: i16 = if self.scanline == -1 {
            5
        } else {
            self.scanline
        };
        let row = line - y as i16;
        if row < 0 || row >= self.sprite_height() {
            self.sprites[g] = SpriteRow::default();
            return;
        }
        if attr & 0x40 != 0 {
            pat_lo = pat_lo.reverse_bits(); // horizontal flip
            pat_hi = pat_hi.reverse_bits();
        }
        // Fetch reloads the counter and shifter; the mode is only changed at
        // dot 339 (if rendering) — a fetch alone leaves a halted counter
        // halted.
        let counting = self.sprites[g].counting;
        self.sprites[g] = SpriteRow {
            counter: x,
            counting,
            pat_lo,
            pat_hi,
            attr,
            is_zero: g == 0 && self.sprite0_cur,
        };
    }

    /// Per-dot sprite counter/shifter update (visible lines, dots 1-256).
    /// Counters tick even with rendering disabled; expired sprites shift
    /// only while rendering is enabled.
    fn clock_sprite_counters(&mut self, rendering: bool) {
        for s in self.sprites[..self.sprite_count].iter_mut() {
            if s.counting {
                if s.counter > 0 {
                    s.counter -= 1;
                }
                if s.counter == 0 {
                    s.counting = false; // halt: drawing starts next dot
                }
            } else if rendering {
                s.pat_lo <<= 1;
                s.pat_hi <<= 1;
            }
        }
    }

    /// Dot 339 with rendering enabled arms the counters; a counter already
    /// at 0 falls straight back to halted (draws at the first pixel).
    fn arm_sprite_counters(&mut self) {
        for s in self.sprites[..self.sprite_count].iter_mut() {
            s.counting = s.counter != 0;
        }
    }

    /// Returns (pattern 0-3, palette index 4-7, behind_bg, is_zero) for first opaque sprite.
    fn sprite_pixel(&self, px: u16) -> (u8, u8, bool, bool) {
        if self.mask & 0x10 == 0 || (px < 8 && self.mask & 0x04 == 0) {
            return (0, 0, false, false);
        }
        for s in &self.sprites[..self.sprite_count] {
            if s.counting {
                continue;
            }
            let pat = ((s.pat_hi >> 7) << 1) | (s.pat_lo >> 7);
            if pat != 0 {
                return (pat, 4 + (s.attr & 3), s.attr & 0x20 != 0, s.is_zero);
            }
        }
        (0, 0, false, false)
    }

    // ---- per-dot tick ----

    pub fn tick(&mut self, cart: &mut dyn Mapper) {
        let visible = (0..240).contains(&self.scanline);
        let prerender = self.scanline == -1;

        // $2001 writes land ~2 dots after the CPU cycle. Disabling rendering
        // mid-line on a sprite-evaluation line seeds OAM corruption with the
        // current secondary OAM address.
        if let Some((val, mut left)) = self.pending_mask.take() {
            if left > 1 {
                left -= 1;
                self.pending_mask = Some((val, left));
            } else {
                let was_on = self.rendering_enabled();
                self.mask = val;
                if was_on
                    && !self.rendering_enabled()
                    && (visible || prerender)
                    && (1..=320).contains(&self.dot)
                {
                    let row = if self.dot <= 64 {
                        self.sec_addr
                    } else if self.dot <= 256 {
                        (self.sec_addr + 3) & !3 & 31
                    } else {
                        self.sec_addr
                    };
                    self.pending_corruption = Some(row);
                }
            }
        }

        let rendering = self.rendering_enabled();

        // $2007 state machine: its Read fires this dot. If the rendering
        // pipeline also fetches this dot, the octal latch feeds back into
        // the fetch address (bus_conflict); either way the read buffer is
        // refilled at the end of the dot from the last bus value.
        let capture_now = self.capture_delay > 0 && {
            self.capture_delay -= 1;
            self.capture_delay == 0
        };
        self.bus_conflict = capture_now;

        // Pending corruption lands on the first rendered dot of a
        // sprite-evaluation line: OAM row 0 overwrites OAM row `seed`.
        if rendering
            && (visible || prerender)
            && let Some(row) = self.pending_corruption.take()
        {
            let row = row as usize & 31;
            for i in 0..8 {
                self.oam[row * 8 + i] = self.oam[i];
            }
            self.secondary_oam[row] = self.secondary_oam[0];
        }

        if (visible || prerender) && rendering {
            self.sprite_pipeline_dot(cart);
        }

        if (visible || prerender) && rendering {
            if (2..=257).contains(&self.dot) || (322..=337).contains(&self.dot) {
                self.shift();
            }
            if (1..=256).contains(&self.dot) || (321..=336).contains(&self.dot) {
                self.bg_fetch(cart);
            }
            if self.dot == 256 {
                self.increment_y();
            }
            if self.dot == 257 {
                self.load_shifters();
                self.copy_horizontal();
            }
            if prerender && (280..=304).contains(&self.dot) {
                self.copy_vertical();
            }
            // OAMADDR is reset during sprite tile fetches on rendered lines.
            if (257..=320).contains(&self.dot) {
                self.oam_addr = 0;
            }
        }

        if prerender && self.dot == 1 {
            self.status &= !(0x80 | 0x40 | 0x20); // clear vblank, sprite 0, overflow
        }

        if visible && (1..=256).contains(&self.dot) {
            self.render_pixel();
            self.clock_sprite_counters(rendering);
        }
        if (visible || prerender) && rendering && self.dot == 339 {
            self.arm_sprite_counters();
        }

        if self.scanline == 241 && self.dot == 1 {
            self.frame_complete = true;
            if !std::mem::take(&mut self.suppress_vbl) {
                self.status |= 0x80;
            }
        }

        if capture_now {
            self.read_buffer = self.last_fetch_val;
            self.bus_conflict = false;
            self.increment_v_after_2007();
        }

        self.dots += 1;
        self.dot += 1;
        // NTSC: odd frames skip the last dot of the pre-render line when
        // rendering is enabled.
        let skip = prerender && self.odd_frame && rendering && self.dot == 340;
        if self.dot > 340 || skip {
            self.dot = 0;
            self.scanline += 1;
            if self.scanline > 260 {
                self.scanline = -1;
                self.odd_frame = !self.odd_frame;
            }
        }
    }

    fn render_pixel(&mut self) {
        let px = self.dot - 1;
        let py = self.scanline as u16;

        // background pixel
        let (mut bg_pat, mut bg_pal) = (0u8, 0u8);
        if self.mask & 0x08 != 0 && !(px < 8 && self.mask & 0x02 == 0) {
            let bit = 15 - self.fine_x as u16;
            bg_pat = ((((self.bg_pat_hi >> bit) & 1) << 1) | ((self.bg_pat_lo >> bit) & 1)) as u8;
            bg_pal = ((((self.bg_attr_hi >> bit) & 1) << 1) | ((self.bg_attr_lo >> bit) & 1)) as u8;
        }

        let (sp_pat, sp_pal, sp_behind, sp_zero) = self.sprite_pixel(px);

        // sprite 0 hit
        if sp_zero && sp_pat != 0 && bg_pat != 0 && px != 255 {
            self.status |= 0x40;
        }

        let palette_idx = match (bg_pat, sp_pat) {
            (0, 0) => 0,
            (0, _) => (sp_pal << 2) | sp_pat,
            (_, 0) => (bg_pal << 2) | bg_pat,
            (_, _) => {
                if sp_behind {
                    (bg_pal << 2) | bg_pat
                } else {
                    (sp_pal << 2) | sp_pat
                }
            }
        };

        let grey = if self.mask & 1 != 0 { 0x30 } else { 0x3F };
        let color = self.palette[Self::palette_index(0x3F00 | palette_idx as u16)] as usize & grey;
        let rgb = NES_PALETTE[color];
        let off = (py as usize * WIDTH + px as usize) * 4;
        self.framebuffer[off] = rgb[0];
        self.framebuffer[off + 1] = rgb[1];
        self.framebuffer[off + 2] = rgb[2];
        self.framebuffer[off + 3] = 0xFF;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::Nrom;

    fn cart() -> Nrom {
        Nrom::new(vec![0; 0x8000], vec![0; 0x2000], Mirroring::Vertical)
    }

    #[test]
    fn loopy_scroll_writes() {
        let mut ppu = Ppu::new();
        let mut c = cart();
        // example from nesdev wiki
        ppu.write_register(0, 0, &mut c);
        ppu.read_register(2, &mut c);
        ppu.write_register(5, 0x7D, &mut c);
        assert_eq!(ppu.t & 0x1F, 0x0F);
        assert_eq!(ppu.fine_x, 5);
        ppu.write_register(5, 0x5E, &mut c);
        // fine_y=6, coarse_y=0x0B, coarse_x=0x0F
        assert_eq!(ppu.t, 0x616F);
        ppu.write_register(6, 0x3D, &mut c);
        ppu.write_register(6, 0xF0, &mut c);
        assert_eq!(ppu.v, 0x3DF0);
        assert_eq!(ppu.v, ppu.t);
    }

    #[test]
    fn status_read_clears_latch_and_vblank() {
        let mut ppu = Ppu::new();
        let mut c = cart();
        ppu.status = 0x80;
        ppu.write_register(6, 0x21, &mut c); // w -> true
        let s = ppu.read_register(2, &mut c);
        assert_eq!(s & 0x80, 0x80);
        assert_eq!(ppu.status & 0x80, 0);
        assert!(!ppu.w);
    }

    #[test]
    fn palette_mirroring() {
        let mut ppu = Ppu::new();
        let mut c = cart();
        ppu.mem_write(0x3F10, 0x22, &mut c);
        assert_eq!(ppu.palette_read(0x3F00), 0x22);
        ppu.mem_write(0x3F04, 0x11, &mut c);
        assert_eq!(ppu.palette_read(0x3F14), 0x11);
    }

    #[test]
    fn ppudata_read_buffered() {
        let mut ppu = Ppu::new();
        let mut c = cart();
        ppu.write_register(6, 0x20, &mut c);
        ppu.write_register(6, 0x00, &mut c);
        ppu.write_register(7, 0xAB, &mut c);
        ppu.write_register(7, 0xCD, &mut c);
        ppu.write_register(6, 0x20, &mut c);
        ppu.write_register(6, 0x00, &mut c);
        let first = ppu.read_register(7, &mut c); // stale buffer
        let second = ppu.read_register(7, &mut c);
        let third = ppu.read_register(7, &mut c);
        assert_eq!(first, 0x00);
        assert_eq!(second, 0xAB);
        assert_eq!(third, 0xCD);
    }

    #[test]
    fn vblank_sets_nmi() {
        let mut ppu = Ppu::new();
        let mut c = cart();
        ppu.write_register(0, 0x80, &mut c);
        // advance to scanline 241 dot 1
        while !(ppu.scanline == 241 && ppu.dot == 2) {
            ppu.tick(&mut c);
        }
        assert!(ppu.status & 0x80 != 0);
        assert!(ppu.nmi_line());
    }
}
