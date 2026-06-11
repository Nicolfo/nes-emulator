use crate::mapper::{Mapper, Mirroring};
use crate::palette::NES_PALETTE;

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 240;

#[derive(Clone, Copy, Default)]
struct SpriteRow {
    x: u8,
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

    ctrl: u8,
    mask: u8,
    status: u8,
    open_bus: u8,

    oam_addr: u8,
    pub oam: [u8; 256],
    sprites: [SpriteRow; 8],
    sprite_count: usize,

    vram: [u8; 0x800],
    palette: [u8; 32],
    read_buffer: u8,

    scanline: i16, // -1 = pre-render, 0..239 visible, 241..260 vblank
    dot: u16,      // 0..340
    nmi_pending: bool,
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
            open_bus: 0,
            oam_addr: 0,
            oam: [0; 256],
            sprites: [SpriteRow::default(); 8],
            sprite_count: 0,
            vram: [0; 0x800],
            palette: [0; 32],
            read_buffer: 0,
            scanline: -1,
            dot: 0,
            nmi_pending: false,
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

    pub fn take_nmi(&mut self) -> bool {
        std::mem::take(&mut self.nmi_pending)
    }

    pub fn oam_addr_for_dma(&self) -> u8 {
        self.oam_addr
    }

    fn rendering_enabled(&self) -> bool {
        self.mask & 0x18 != 0
    }

    // ---- register interface ($2000-$2007) ----

    pub fn read_register(&mut self, reg: u16, cart: &mut dyn Mapper) -> u8 {
        match reg & 7 {
            2 => {
                let res = (self.status & 0xE0) | (self.open_bus & 0x1F);
                self.status &= !0x80; // clear vblank
                self.w = false;
                res
            }
            4 => self.oam[self.oam_addr as usize],
            7 => {
                let addr = self.v & 0x3FFF;
                let res;
                if addr >= 0x3F00 {
                    res = self.palette_read(addr);
                    // buffer gets the nametable byte "underneath" the palette
                    self.read_buffer = self.mem_read(addr & 0x2FFF, cart);
                } else {
                    res = self.read_buffer;
                    self.read_buffer = self.mem_read(addr, cart);
                }
                self.v = self.v.wrapping_add(self.vram_increment()) & 0x7FFF;
                res
            }
            _ => self.open_bus,
        }
    }

    pub fn write_register(&mut self, reg: u16, val: u8, cart: &mut dyn Mapper) {
        self.open_bus = val;
        match reg & 7 {
            0 => {
                let old_nmi = self.ctrl & 0x80;
                self.ctrl = val;
                self.t = (self.t & !0x0C00) | (((val & 3) as u16) << 10);
                // NMI fires immediately if enabled during vblank
                if old_nmi == 0 && val & 0x80 != 0 && self.status & 0x80 != 0 {
                    self.nmi_pending = true;
                }
            }
            1 => self.mask = val,
            3 => self.oam_addr = val,
            4 => {
                self.oam[self.oam_addr as usize] = val;
                self.oam_addr = self.oam_addr.wrapping_add(1);
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
                self.v = self.v.wrapping_add(self.vram_increment()) & 0x7FFF;
            }
            _ => {}
        }
    }

    fn vram_increment(&self) -> u16 {
        if self.ctrl & 0x04 != 0 { 32 } else { 1 }
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
        self.bg_pat_lo <<= 1;
        self.bg_pat_hi <<= 1;
        self.bg_attr_lo <<= 1;
        self.bg_attr_hi <<= 1;
    }

    fn bg_fetch(&mut self, cart: &mut dyn Mapper) {
        match (self.dot - 1) % 8 {
            0 => {
                self.load_shifters();
                self.nt_latch = self.mem_read(0x2000 | (self.v & 0x0FFF), cart);
            }
            2 => {
                let addr = 0x23C0
                    | (self.v & 0x0C00)
                    | ((self.v >> 4) & 0x38)
                    | ((self.v >> 2) & 0x07);
                let mut at = self.mem_read(addr, cart);
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
                self.pat_lo_latch = self.mem_read(base + fine_y, cart);
            }
            6 => {
                let fine_y = (self.v >> 12) & 7;
                let base = ((self.ctrl as u16 & 0x10) << 8) | ((self.nt_latch as u16) << 4);
                self.pat_hi_latch = self.mem_read(base + fine_y + 8, cart);
            }
            7 => self.increment_coarse_x(),
            _ => {}
        }
    }

    // ---- sprites ----

    fn sprite_height(&self) -> i16 {
        if self.ctrl & 0x20 != 0 { 16 } else { 8 }
    }

    /// Evaluate + fetch sprites for the next scanline (hybrid: batched at dot 257).
    fn evaluate_sprites(&mut self, cart: &mut dyn Mapper) {
        let next = self.scanline + 1;
        self.sprite_count = 0;
        let height = self.sprite_height();
        let mut overflow = false;
        for i in 0..64 {
            let y = self.oam[i * 4] as i16;
            // OAM Y is top-1: sprite occupies scanlines y+1 .. y+height
            let row = next - y - 1;
            if row < 0 || row >= height {
                continue;
            }
            if self.sprite_count == 8 {
                overflow = true;
                break;
            }
            let tile = self.oam[i * 4 + 1];
            let attr = self.oam[i * 4 + 2];
            let x = self.oam[i * 4 + 3];
            let mut row = row;
            if attr & 0x80 != 0 {
                row = height - 1 - row; // vertical flip
            }
            let pat_addr = if height == 16 {
                let table = ((tile & 1) as u16) << 12;
                let mut t = (tile & 0xFE) as u16;
                if row >= 8 {
                    t += 1;
                }
                table | (t << 4) | (row as u16 & 7)
            } else {
                ((self.ctrl as u16 & 0x08) << 9) | ((tile as u16) << 4) | row as u16
            };
            let mut pat_lo = cart.ppu_read(pat_addr);
            let mut pat_hi = cart.ppu_read(pat_addr + 8);
            if attr & 0x40 != 0 {
                pat_lo = pat_lo.reverse_bits(); // horizontal flip
                pat_hi = pat_hi.reverse_bits();
            }
            self.sprites[self.sprite_count] =
                SpriteRow { x, pat_lo, pat_hi, attr, is_zero: i == 0 };
            self.sprite_count += 1;
        }
        if overflow {
            self.status |= 0x20;
        }
    }

    /// Returns (pattern 0-3, palette index 4-7, behind_bg, is_zero) for first opaque sprite.
    fn sprite_pixel(&self, px: u16) -> (u8, u8, bool, bool) {
        if self.mask & 0x10 == 0 || (px < 8 && self.mask & 0x04 == 0) {
            return (0, 0, false, false);
        }
        for s in &self.sprites[..self.sprite_count] {
            let off = px.wrapping_sub(s.x as u16);
            if off >= 8 {
                continue;
            }
            let bit = 7 - off;
            let pat = (((s.pat_hi >> bit) & 1) << 1) | ((s.pat_lo >> bit) & 1);
            if pat != 0 {
                return (pat, 4 + (s.attr & 3), s.attr & 0x20 != 0, s.is_zero);
            }
        }
        (0, 0, false, false)
    }

    // ---- per-dot tick ----

    pub fn tick(&mut self, cart: &mut dyn Mapper) {
        let rendering = self.rendering_enabled();
        let visible = (0..240).contains(&self.scanline);
        let prerender = self.scanline == -1;

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
            if self.dot == 257 && self.scanline < 239 {
                self.evaluate_sprites(cart);
            }
        }

        if prerender && self.dot == 1 {
            self.status &= !(0x80 | 0x40 | 0x20); // clear vblank, sprite 0, overflow
        }

        if visible && (1..=256).contains(&self.dot) {
            self.render_pixel();
        }

        if self.scanline == 241 && self.dot == 1 {
            self.status |= 0x80;
            self.frame_complete = true;
            if self.ctrl & 0x80 != 0 {
                self.nmi_pending = true;
            }
        }

        self.dot += 1;
        if self.dot > 340 {
            self.dot = 0;
            self.scanline += 1;
            if self.scanline > 260 {
                self.scanline = -1;
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

        let color = self.palette[Self::palette_index(0x3F00 | palette_idx as u16)] as usize & 0x3F;
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
        assert!(ppu.take_nmi());
    }
}
