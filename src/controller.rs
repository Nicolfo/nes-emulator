// Button bits: 0=A 1=B 2=Select 3=Start 4=Up 5=Down 6=Left 7=Right
pub const BTN_A: u8 = 0x01;
pub const BTN_B: u8 = 0x02;
pub const BTN_SELECT: u8 = 0x04;
pub const BTN_START: u8 = 0x08;
pub const BTN_UP: u8 = 0x10;
pub const BTN_DOWN: u8 = 0x20;
pub const BTN_LEFT: u8 = 0x40;
pub const BTN_RIGHT: u8 = 0x80;

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Controller {
    strobe: bool,
    shift: u8,
    pub state: u8,
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
        }
    }

    pub fn read(&mut self) -> u8 {
        if self.strobe {
            return 0x40 | (self.state & 1);
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
}
