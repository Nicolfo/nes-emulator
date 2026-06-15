//! Optional physical-gamepad input via `gilrs`. The first connected pad drives
//! player 1 and the second drives player 2; directions come from either the
//! D-pad or the left analog stick. If gilrs can't initialise (no input backend
//! on the host) the frontend silently stays keyboard-only.

use gilrs::{Axis, Button, EventType, Gamepad, GamepadId, Gilrs};

use nes_emulator::controller::{
    BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP,
};

/// Tracks connected pads in plug order and maps their live state to the two
/// NES controller button masks.
pub struct Gamepads {
    gilrs: Gilrs,
    /// Connected pad ids in the order they appeared; slot 0 = P1, slot 1 = P2.
    pads: Vec<GamepadId>,
}

impl Gamepads {
    /// `None` when gilrs can't start; the frontend then stays keyboard-only.
    pub fn new() -> Option<Self> {
        let gilrs = Gilrs::new().ok()?;
        let pads = gilrs
            .gamepads()
            .filter(|(_, gp)| gp.is_connected())
            .map(|(id, _)| id)
            .collect();
        Some(Gamepads { gilrs, pads })
    }

    /// Drain pending events (refreshing button state and tracking hot-plug),
    /// then return the current button masks for player 1 and player 2. A slot
    /// with no connected pad reads as 0 (nothing pressed).
    pub fn poll(&mut self) -> [u8; 2] {
        while let Some(ev) = self.gilrs.next_event() {
            match ev.event {
                EventType::Connected => {
                    if !self.pads.contains(&ev.id) {
                        self.pads.push(ev.id);
                    }
                }
                EventType::Disconnected => self.pads.retain(|&id| id != ev.id),
                _ => {}
            }
        }
        let mut masks = [0u8; 2];
        for (slot, mask) in masks.iter_mut().enumerate() {
            if let Some(&id) = self.pads.get(slot) {
                *mask = buttons(&self.gilrs.gamepad(id));
            }
        }
        masks
    }
}

/// Map one pad's live state to an NES button mask. South/East are the bottom
/// and right face buttons (A/B); directions come from the D-pad or, past a
/// deadzone, the left analog stick.
fn buttons(gp: &Gamepad) -> u8 {
    const THRESHOLD: f32 = 0.5; // analog-stick deflection that counts as held
    let mut m = 0u8;
    if gp.is_pressed(Button::South) {
        m |= BTN_A;
    }
    if gp.is_pressed(Button::East) {
        m |= BTN_B;
    }
    if gp.is_pressed(Button::Start) {
        m |= BTN_START;
    }
    if gp.is_pressed(Button::Select) {
        m |= BTN_SELECT;
    }
    // gilrs reports LeftStickY positive-up.
    let (x, y) = (gp.value(Axis::LeftStickX), gp.value(Axis::LeftStickY));
    if gp.is_pressed(Button::DPadUp) || y > THRESHOLD {
        m |= BTN_UP;
    }
    if gp.is_pressed(Button::DPadDown) || y < -THRESHOLD {
        m |= BTN_DOWN;
    }
    if gp.is_pressed(Button::DPadLeft) || x < -THRESHOLD {
        m |= BTN_LEFT;
    }
    if gp.is_pressed(Button::DPadRight) || x > THRESHOLD {
        m |= BTN_RIGHT;
    }
    m
}
