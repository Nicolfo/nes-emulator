//! NES Zapper light gun (controller port 2, $4017).
//!
//! The Zapper reports two bits: D4 is the trigger (1 = pulled) and D3 the
//! light sensor (0 = light detected). The photodiode only "sees" light for a
//! short while after the CRT beam sweeps past the spot it is aimed at, so a
//! read checks the pixels around the aim point that the PPU has drawn within
//! the last few scanlines of the *current* frame. Games exploit this: Duck
//! Hunt blanks the screen and draws a white box over each target on the frame
//! after the trigger, then checks the sensor mid-frame.

use serde::{Deserialize, Serialize};

use crate::ppu::{HEIGHT, Ppu, WIDTH};

/// How many scanlines after the beam passes a bright pixel the sensor still
/// reports light (the photodiode's decay).
const LIGHT_LINES: i32 = 20;
/// Radius (pixels) around the aim point the sensor collects light from.
const SENSE_RADIUS: i32 = 2;
/// Minimum average RGB brightness that counts as light.
const BRIGHTNESS: u32 = 85;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Zapper {
    /// Aim point in NES pixels; negative/out of range = pointing off screen.
    pub x: i32,
    pub y: i32,
    pub trigger: bool,
}

impl Zapper {
    /// Value of a $4017 read (D3/D4 only; the bus adds open-bus bits).
    pub fn read(&self, ppu: &Ppu) -> u8 {
        let trigger = if self.trigger { 0x10 } else { 0 };
        let dark = if self.light_sensed(ppu) { 0 } else { 0x08 };
        trigger | dark
    }

    pub fn light_sensed(&self, ppu: &Ppu) -> bool {
        let (beam_y, beam_x) = (ppu.scanline as i32, ppu.dot as i32 - 1);
        // The pre-render line (-1) starts a new frame: nothing lit yet. In
        // vblank the last visible rows are still within the photodiode's
        // decay window, so they keep reading as light for a few lines.
        if beam_y < 0 || !self.on_screen() {
            return false;
        }
        for py in self.y - SENSE_RADIUS..=self.y + SENSE_RADIUS {
            if !(0..HEIGHT as i32).contains(&py) {
                continue;
            }
            // Only rows the beam has drawn this frame, recently enough that
            // the photodiode hasn't decayed.
            let lag = beam_y - py;
            if !(0..=LIGHT_LINES).contains(&lag) {
                continue;
            }
            for px in self.x - SENSE_RADIUS..=self.x + SENSE_RADIUS {
                if !(0..WIDTH as i32).contains(&px) || (lag == 0 && px > beam_x) {
                    continue;
                }
                let i = (py as usize * WIDTH + px as usize) * 4;
                let fb = &ppu.framebuffer;
                let sum = fb[i] as u32 + fb[i + 1] as u32 + fb[i + 2] as u32;
                if sum >= BRIGHTNESS * 3 {
                    return true;
                }
            }
        }
        false
    }

    fn on_screen(&self) -> bool {
        (0..WIDTH as i32).contains(&self.x) && (0..HEIGHT as i32).contains(&self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppu_at(scanline: i16, dot: u16) -> Ppu {
        let mut p = Ppu::new();
        p.scanline = scanline;
        p.dot = dot;
        p
    }

    fn paint(p: &mut Ppu, x: usize, y: usize, rgb: u8) {
        let i = (y * WIDTH + x) * 4;
        p.framebuffer[i..i + 3].fill(rgb);
    }

    #[test]
    fn trigger_and_dark_bits() {
        let p = ppu_at(100, 10);
        let mut z = Zapper {
            x: 50,
            y: 50,
            trigger: false,
        };
        assert_eq!(z.read(&p), 0x08); // dark, not pulled
        z.trigger = true;
        assert_eq!(z.read(&p), 0x18);
    }

    #[test]
    fn senses_bright_pixel_just_behind_the_beam() {
        let mut p = ppu_at(105, 200);
        paint(&mut p, 50, 100, 0xFC);
        let z = Zapper {
            x: 50,
            y: 100,
            trigger: false,
        };
        assert_eq!(z.read(&p) & 0x08, 0, "light expected");
    }

    #[test]
    fn ignores_pixels_the_beam_has_not_reached_or_long_passed() {
        let mut p = ppu_at(90, 200); // beam above the target
        paint(&mut p, 50, 100, 0xFC);
        let z = Zapper {
            x: 50,
            y: 100,
            trigger: false,
        };
        assert_eq!(z.read(&p) & 0x08, 0x08);
        p.scanline = 100 + LIGHT_LINES as i16 + SENSE_RADIUS as i16 + 1; // decayed
        assert_eq!(z.read(&p) & 0x08, 0x08);
        p.scanline = -1; // pre-render: new frame, nothing lit
        assert_eq!(z.read(&p) & 0x08, 0x08);
    }

    #[test]
    fn bottom_rows_stay_lit_into_early_vblank() {
        let mut p = ppu_at(245, 10);
        paint(&mut p, 50, 235, 0xFC);
        let z = Zapper {
            x: 50,
            y: 235,
            trigger: false,
        };
        assert_eq!(z.read(&p) & 0x08, 0, "decay window spans into vblank");
        p.scanline = 270;
        assert_eq!(z.read(&p) & 0x08, 0x08, "decayed by line 270");
    }

    #[test]
    fn off_screen_never_sees_light() {
        let mut p = ppu_at(10, 100);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                paint(&mut p, x, y, 0xFF);
            }
        }
        let z = Zapper {
            x: -1,
            y: -1,
            trigger: true,
        };
        assert_eq!(z.read(&p), 0x18);
    }
}
