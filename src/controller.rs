// Button bits: 0=A 1=B 2=Select 3=Start 4=Up 5=Down 6=Left 7=Right
pub const BTN_A: u8 = 0x01;
pub const BTN_B: u8 = 0x02;
pub const BTN_SELECT: u8 = 0x04;
pub const BTN_START: u8 = 0x08;
pub const BTN_UP: u8 = 0x10;
pub const BTN_DOWN: u8 = 0x20;
pub const BTN_LEFT: u8 = 0x40;
pub const BTN_RIGHT: u8 = 0x80;

/// What is plugged into the two controller ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum InputDevice {
    /// A standard joypad in each port.
    #[default]
    Standard,
    /// NES Four Score: each port carries two joypads (P1+P3 on $4016,
    /// P2+P4 on $4017) followed by an 8-bit signature.
    FourScore,
    /// Joypad in port 1, Zapper light gun in port 2.
    Zapper,
}

/// Four Score signature bytes, shifted out LSB first after the two pads'
/// 16 bits. Read MSB-first (the usual ROL loop) they spell $10 on $4016 and
/// $20 on $4017: bits 17-24 read 0,0,0,1,0,0,0,0 and 0,0,1,0,0,0,0,0.
pub const FOUR_SCORE_SIG_4016: u8 = 0x08;
pub const FOUR_SCORE_SIG_4017: u8 = 0x04;

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Controller {
    strobe: bool,
    shift: u8,
    pub state: u8,
    /// Buttons of the second pad on this port (P3/P4) behind a Four Score.
    #[serde(default)]
    pub state_hi: u8,
    /// 24-bit Four Score shift register (pad, second pad, signature).
    #[serde(default)]
    shift24: u32,
    /// Port wiring, owned by the host configuration rather than the machine
    /// state: savestates don't carry it (see `Nes::load_state`).
    #[serde(skip)]
    four_score: bool,
    #[serde(skip)]
    signature: u8,
}

impl Controller {
    /// Writes only set the strobe line level; the shift register reloads on
    /// "put" CPU cycles while the line is high (see `clock_put_cycle`).
    pub fn write(&mut self, val: u8) {
        self.strobe = val & 1 != 0;
    }

    /// Called by the bus once per "put" CPU cycle.
    pub fn clock_put_cycle(&mut self) {
        if self.strobe {
            self.shift = self.state;
            self.shift24 =
                self.state as u32 | (self.state_hi as u32) << 8 | (self.signature as u32) << 16;
        }
    }

    pub fn read(&mut self) -> u8 {
        if self.strobe {
            return 0x40 | (self.state & 1);
        }
        if self.four_score {
            // After all 24 bits the register keeps shifting in 1s.
            let bit = (self.shift24 & 1) as u8;
            self.shift24 = (self.shift24 >> 1) | 0x80_0000;
            return 0x40 | bit;
        }
        let bit = self.shift & 1;
        self.shift = (self.shift >> 1) | 0x80;
        0x40 | bit
    }

    pub fn set_button(&mut self, mask: u8, pressed: bool) {
        if pressed {
            self.state |= mask;
        } else {
            self.state &= !mask;
        }
    }

    /// Wire this port through a Four Score (`Some(signature)`) or directly
    /// to a single pad (`None`).
    pub fn set_four_score(&mut self, signature: Option<u8>) {
        self.four_score = signature.is_some();
        self.signature = signature.unwrap_or(0);
    }

    /// The host-owned port wiring, to carry across a savestate restore.
    pub fn wiring(&self) -> Option<u8> {
        self.four_score.then_some(self.signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_sequence() {
        let mut c = Controller::default();
        c.set_button(BTN_A, true);
        c.set_button(BTN_START, true);
        c.write(1);
        c.clock_put_cycle();
        c.write(0);
        let bits: Vec<u8> = (0..10).map(|_| c.read() & 1).collect();
        // A, B, Select, Start, Up, Down, Left, Right, then 1s
        assert_eq!(bits, vec![1, 0, 0, 1, 0, 0, 0, 0, 1, 1]);
    }

    #[test]
    fn strobe_high_returns_a() {
        let mut c = Controller::default();
        c.set_button(BTN_A, true);
        c.write(1);
        assert_eq!(c.read() & 1, 1);
        assert_eq!(c.read() & 1, 1); // doesn't advance while strobe high
    }

    fn read_bits(c: &mut Controller, n: usize) -> Vec<u8> {
        c.write(1);
        c.clock_put_cycle();
        c.write(0);
        (0..n).map(|_| c.read() & 1).collect()
    }

    /// Accumulate 8 serial bits the way games do (LSR A / ROL tmp), so the
    /// first bit read lands in bit 7.
    fn rol_byte(bits: &[u8]) -> u8 {
        bits.iter().fold(0u8, |acc, &b| (acc << 1) | b)
    }

    #[test]
    fn four_score_reports_both_pads_and_signature() {
        let mut p1 = Controller::default();
        p1.set_four_score(Some(FOUR_SCORE_SIG_4016));
        p1.state = BTN_A;
        p1.state_hi = BTN_START | BTN_RIGHT;
        let bits = read_bits(&mut p1, 26);
        assert_eq!(&bits[0..8], &[1, 0, 0, 0, 0, 0, 0, 0]); // P1: A
        assert_eq!(&bits[8..16], &[0, 0, 0, 1, 0, 0, 0, 1]); // P3: Start, Right
        assert_eq!(rol_byte(&bits[16..24]), 0x10);
        assert_eq!(&bits[24..], &[1, 1]); // then 1s

        let mut p2 = Controller::default();
        p2.set_four_score(Some(FOUR_SCORE_SIG_4017));
        let bits = read_bits(&mut p2, 24);
        assert_eq!(rol_byte(&bits[16..24]), 0x20);
    }

    #[test]
    fn wiring_survives_serde_skip() {
        let mut c = Controller::default();
        c.set_four_score(Some(FOUR_SCORE_SIG_4016));
        let json = serde_json::to_vec(&c).unwrap();
        let restored: Controller = serde_json::from_slice(&json).unwrap();
        // Wiring is host config, not machine state.
        assert_eq!(restored.wiring(), None);
        assert_eq!(c.wiring(), Some(FOUR_SCORE_SIG_4016));
    }
}
