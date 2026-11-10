//! Famicom Disk System: the RAM adapter (32KB PRG RAM, 8KB CHR RAM, the 8KB
//! BIOS at $E000, a CPU-cycle timer IRQ, mirroring control, expansion sound)
//! plus the disk drive.
//!
//! Disk images come in the `.fds` layout: each side is 65500 bytes of block
//! data with no gaps or CRCs. The drive works on a second, "gapped" stream
//! per side that looks like what the head actually sees - a long leading gap,
//! then for each block a $80 start mark, the block bytes, a CRC-16 and a
//! 976-bit gap - because the BIOS synchronises on the gap/mark sequence. The
//! head moves one byte every ~149 CPU cycles while the motor runs.
//!
//! Writes the game makes land in the gapped stream and are folded back into
//! the `.fds` layout whenever a write session ends (back to read mode, motor
//! off, end of side, eject). That layout is exposed through
//! [`Mapper::prg_ram`] so the frontend's `.sav` plumbing persists disk saves.

use serde::{Deserialize, Serialize};

use super::fds_audio::FdsAudio;
use super::{Mapper, Mirroring};

/// Bytes per disk side in the `.fds` layout.
pub const SIDE_SIZE: usize = 65500;
/// fwNES header length.
const HEADER_LEN: usize = 16;
/// Leading gap before the first block: 28300 bits.
const LEAD_GAP: usize = 28300 / 8;
/// Gap after each block's CRC: 976 bits.
const BLOCK_GAP: usize = 976 / 8;
/// CPU cycles per byte under the head (96.4 kbit/s at 1.79 MHz).
const BYTE_CYCLES: u32 = 149;
/// CPU cycles the head takes to get back to the start of the side.
const REWIND_CYCLES: u32 = 50_000;
/// How long a side change keeps the drive empty, so the BIOS sees an eject
/// before the new side appears (~1 s).
const INSERT_DELAY: u32 = 1_789_773;
/// Extra trailing gap past the converted data, room for files a game writes.
const TAIL_SLACK: usize = 4096;

/// Split a disk image into `.fds`-layout sides. Accepts a fwNES file
/// (16-byte `FDS\x1A` header) or a headerless dump starting with the disk
/// info block. A short final side is zero-padded.
pub fn parse_disk_image(data: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let (body, declared) = if data.len() >= HEADER_LEN && &data[0..4] == b"FDS\x1A" {
        (&data[HEADER_LEN..], Some(data[4] as usize))
    } else if data.len() >= 15 && data[0] == 0x01 && &data[1..15] == b"*NINTENDO-HVC*" {
        (data, None)
    } else {
        return Err("not an FDS disk image".into());
    };
    let found = body.len().div_ceil(SIDE_SIZE);
    let sides = match declared {
        Some(0) | None => found,
        Some(n) => n.min(found.max(1)),
    };
    if sides == 0 {
        return Err("FDS image has no disk sides".into());
    }
    if sides > 16 {
        return Err(format!("FDS image claims {sides} sides"));
    }
    Ok((0..sides)
        .map(|i| {
            let start = (i * SIDE_SIZE).min(body.len());
            let end = ((i + 1) * SIDE_SIZE).min(body.len());
            let mut side = body[start..end].to_vec();
            side.resize(SIDE_SIZE, 0);
            side
        })
        .collect())
}

/// One step of the drive's CRC-16 (poly $8408, LSB first). Running it over a
/// block's $80 mark, the block bytes and two zero bytes yields the CRC that
/// follows the block on disk.
fn crc_step(crc: u16, val: u8) -> u16 {
    let mut crc = crc;
    let mut n = 0x01u8;
    loop {
        let carry = crc & 1 != 0;
        crc >>= 1;
        if carry {
            crc ^= 0x8408;
        }
        if val & n != 0 {
            crc ^= 0x8000;
        }
        if n == 0x80 {
            return crc;
        }
        n <<= 1;
    }
}

/// Length of the block starting at `data[i]`, given the size declared by the
/// preceding file header. None for an unknown block type (end of data).
fn block_len(block_type: u8, file_size: usize) -> Option<usize> {
    match block_type {
        1 => Some(56),
        2 => Some(2),
        3 => Some(16),
        4 => Some(1 + file_size),
        _ => None,
    }
}

/// `.fds` side -> the gapped stream the head reads.
fn side_to_stream(side: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; LEAD_GAP];
    let mut i = 0;
    let mut file_size = 0;
    while i < side.len() {
        let Some(len) = block_len(side[i], file_size) else {
            break;
        };
        if side[i] == 3 && i + 14 < side.len() {
            file_size = side[i + 13] as usize | (side[i + 14] as usize) << 8;
        }
        let end = (i + len).min(side.len());
        out.push(0x80);
        let mut crc = crc_step(0, 0x80);
        for &b in &side[i..end] {
            out.push(b);
            crc = crc_step(crc, b);
        }
        crc = crc_step(crc_step(crc, 0), 0);
        out.push(crc as u8);
        out.push((crc >> 8) as u8);
        out.extend(std::iter::repeat_n(0, BLOCK_GAP));
        i = end;
    }
    // Free space on the side becomes trailing gap, plus slack for overhead
    // of blocks a game may append.
    out.resize(out.len() + (SIDE_SIZE - i.min(SIDE_SIZE)) + TAIL_SLACK, 0);
    out
}

/// Gapped stream -> `.fds` side: collect each block after a $80 mark,
/// skipping marks, CRCs and gaps.
fn stream_to_side(stream: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(SIDE_SIZE);
    let mut i = 0;
    let mut file_size = 0;
    let mut in_gap = true;
    while i < stream.len() && out.len() < SIDE_SIZE {
        if in_gap {
            in_gap = stream[i] != 0x80;
            i += 1;
            continue;
        }
        let len = block_len(stream[i], file_size).unwrap_or(1);
        if stream[i] == 3 && i + 14 < stream.len() {
            file_size = stream[i + 13] as usize | (stream[i + 14] as usize) << 8;
        }
        let end = (i + len).min(stream.len());
        out.extend_from_slice(&stream[i..end]);
        i = end + 2; // skip CRC
        in_gap = true;
    }
    out.resize(SIDE_SIZE, 0);
    out
}

/// `serde` helper: `Vec<u8>` as a hex string, about half the size of a JSON
/// number array - disk sides are ~70KB each.
mod hex {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(v.len() * 2);
        for &b in v {
            out.push(DIGITS[(b >> 4) as usize] as char);
            out.push(DIGITS[(b & 15) as usize] as char);
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        let digit = |c: u8| -> Result<u8, D::Error> {
            match c {
                b'0'..=b'9' => Ok(c - b'0'),
                b'a'..=b'f' => Ok(c - b'a' + 10),
                b'A'..=b'F' => Ok(c - b'A' + 10),
                _ => Err(serde::de::Error::custom("bad hex digit")),
            }
        };
        let b = s.as_bytes();
        if b.len() % 2 != 0 {
            return Err(serde::de::Error::custom("odd hex length"));
        }
        b.chunks(2)
            .map(|p| Ok(digit(p[0])? << 4 | digit(p[1])?))
            .collect()
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Stream(#[serde(with = "hex")] Vec<u8>);

#[derive(Serialize, Deserialize)]
pub struct Fds {
    /// The 8KB BIOS (`disksys.rom`), from the frontend, never in a state.
    #[serde(skip)]
    prg: Vec<u8>,
    /// `.fds`-layout disk, rebuilt from `streams`; what `.sav` persists.
    #[serde(skip)]
    disk: Vec<u8>,
    streams: Vec<Stream>,
    /// Side whose stream has unsynced writes.
    dirty: Option<usize>,
    #[serde(with = "hex")]
    ram: Vec<u8>,
    #[serde(with = "hex")]
    chr: Vec<u8>,

    // Timer IRQ
    irq_reload: u16,
    irq_counter: u16,
    irq_enabled: bool,
    irq_repeat: bool,
    timer_irq: bool,

    // $4023 I/O enables
    disk_regs: bool,
    sound_regs: bool,

    // $4025 control
    motor_on: bool,
    reset_transfer: bool,
    read_mode: bool,
    horizontal: bool,
    crc_control: bool,
    disk_ready: bool,
    disk_irq_enabled: bool,
    ext_write: u8,

    // Drive
    write_data: u8,
    read_data: u8,
    transfer_complete: bool,
    disk_irq: bool,
    end_of_head: bool,
    scanning: bool,
    gap_ended: bool,
    prev_crc_control: bool,
    crc: u16,
    delay: u32,
    position: usize,
    side: Option<usize>,
    pending_side: Option<usize>,
    insert_delay: u32,
    /// Independent block-CRC checker for the $4030 CRC-error flag: CRC over
    /// the block from its start mark, the bytes left to see it reach zero
    /// once the game raises CRC control, and the latched error.
    #[serde(default)]
    check_crc: u16,
    #[serde(default)]
    crc_window: u8,
    #[serde(default)]
    crc_error: bool,
    /// Automatic side selection from the BIOS disk check (see
    /// [`Mapper::fds_bios_disk_check`]); switched off for disks whose sides
    /// can't be told apart.
    #[serde(default = "default_true")]
    auto_insert: bool,

    audio: FdsAudio,
}

fn default_true() -> bool {
    true
}

/// Offset of the 10-byte disk ID (manufacturer, game name, type, version,
/// side, disk number, disk type, ...) in a side: after the block code and
/// the 14-character "*NINTENDO-HVC*" string.
const DISK_ID: usize = 15;
/// Bytes the BIOS compares, with $FF in the expected ID as a wildcard.
const DISK_ID_LEN: usize = 10;

impl Fds {
    /// `bios` must be 8KB; `sides` are `.fds`-layout sides (see
    /// [`parse_disk_image`]). Side A of disk 1 starts inserted.
    pub fn new(bios: Vec<u8>, sides: Vec<Vec<u8>>) -> Self {
        let streams = sides.iter().map(|s| Stream(side_to_stream(s))).collect();
        let disk = sides.concat();
        Fds {
            prg: bios,
            disk,
            streams,
            dirty: None,
            ram: vec![0; 0x8000],
            chr: vec![0; 0x2000],
            irq_reload: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_repeat: false,
            timer_irq: false,
            disk_regs: true,
            sound_regs: true,
            motor_on: false,
            reset_transfer: false,
            read_mode: true,
            horizontal: false,
            crc_control: false,
            disk_ready: false,
            disk_irq_enabled: false,
            ext_write: 0,
            write_data: 0,
            read_data: 0,
            transfer_complete: false,
            disk_irq: false,
            end_of_head: true,
            scanning: false,
            gap_ended: false,
            prev_crc_control: false,
            crc: 0,
            delay: 0,
            position: 0,
            side: Some(0),
            pending_side: None,
            insert_delay: 0,
            check_crc: 0,
            crc_window: 0,
            crc_error: false,
            auto_insert: true,
            audio: FdsAudio::new(),
        }
    }

    /// Fold pending writes in the gapped stream back into the `.fds` layout.
    fn sync_disk(&mut self) {
        if let Some(s) = self.dirty.take() {
            let side = stream_to_side(&self.streams[s].0);
            self.disk[s * SIDE_SIZE..(s + 1) * SIDE_SIZE].copy_from_slice(&side);
        }
    }

    /// Regenerate every gapped stream from the `.fds` layout.
    fn rebuild_streams(&mut self) {
        self.dirty = None;
        for (i, side) in self.disk.chunks(SIDE_SIZE).enumerate() {
            self.streams[i] = Stream(side_to_stream(side));
        }
    }

    fn clock_timer(&mut self) {
        if !self.irq_enabled {
            return;
        }
        if self.irq_counter == 0 {
            self.timer_irq = true;
            self.irq_counter = self.irq_reload;
            if !self.irq_repeat {
                self.irq_enabled = false;
            }
        } else {
            self.irq_counter -= 1;
        }
    }

    fn clock_drive(&mut self) {
        if self.insert_delay > 0 {
            self.insert_delay -= 1;
            if self.insert_delay == 0 {
                self.side = self.pending_side.take();
            }
        }
        let Some(side) = self.side else {
            self.end_of_head = true;
            self.scanning = false;
            return;
        };
        if !self.motor_on {
            self.end_of_head = true;
            self.scanning = false;
            return;
        }
        if self.reset_transfer && !self.scanning {
            return;
        }
        if self.end_of_head {
            self.delay = REWIND_CYCLES;
            self.end_of_head = false;
            self.position = 0;
            self.gap_ended = false;
            return;
        }
        if self.delay > 0 {
            self.delay -= 1;
            return;
        }
        // A side swap or a restored state can leave the head past the end
        // of this side's stream; rewind instead of reading out of range.
        if self.position >= self.streams[side].0.len() {
            self.end_of_head = true;
            return;
        }

        self.scanning = true;
        let mut need_irq = self.disk_irq_enabled;
        if self.read_mode {
            let data = self.streams[side].0[self.position];
            if !self.prev_crc_control {
                self.crc = crc_step(self.crc, data);
            }
            if !self.disk_ready {
                self.gap_ended = false;
                self.crc = 0;
            } else if data != 0 && !self.gap_ended {
                // The $80 start mark ends the gap; it is latched but does
                // not raise the transfer IRQ.
                self.gap_ended = true;
                need_irq = false;
                self.check_crc = 0;
                self.crc_window = 0;
                self.crc_error = false;
            }
            if self.gap_ended {
                self.check_crc = crc_step(self.check_crc, data);
                if self.crc_control && !self.prev_crc_control {
                    // CRC check requested: the block's two CRC bytes are
                    // at most a few bytes away.
                    self.crc_window = 3;
                }
                if self.crc_window > 0 {
                    if self.check_crc == 0 {
                        self.crc_window = 0;
                    } else {
                        self.crc_window -= 1;
                        self.crc_error = self.crc_window == 0;
                    }
                }
            }
            if self.gap_ended {
                self.transfer_complete = true;
                self.read_data = data;
                if need_irq {
                    self.disk_irq = true;
                }
            }
        } else {
            let mut data = 0;
            if !self.crc_control {
                self.transfer_complete = true;
                data = self.write_data;
                if need_irq {
                    self.disk_irq = true;
                }
            }
            if !self.disk_ready {
                data = 0;
            }
            if !self.crc_control {
                self.crc = crc_step(self.crc, data);
            } else {
                if !self.prev_crc_control {
                    self.crc = crc_step(crc_step(self.crc, 0), 0);
                }
                data = self.crc as u8;
                self.crc >>= 8;
            }
            // The write head trails the read head by two bytes.
            if self.position >= 2 {
                let slot = &mut self.streams[side].0[self.position - 2];
                if *slot != data {
                    *slot = data;
                    self.dirty = Some(side);
                }
            }
            self.gap_ended = false;
        }
        self.prev_crc_control = self.crc_control;

        self.position += 1;
        if self.position >= self.streams[side].0.len() {
            self.motor_on = false;
            self.end_of_head = true;
            self.sync_disk();
        } else {
            self.delay = BYTE_CYCLES;
        }
    }

    fn write_control(&mut self, val: u8) {
        let was_writing = !self.read_mode;
        self.motor_on = val & 0x01 != 0;
        self.reset_transfer = val & 0x02 != 0;
        self.read_mode = val & 0x04 != 0;
        self.horizontal = val & 0x08 != 0;
        self.crc_control = val & 0x10 != 0;
        self.disk_ready = val & 0x40 != 0;
        self.disk_irq_enabled = val & 0x80 != 0;
        self.disk_irq = false;
        if (was_writing && self.read_mode) || !self.motor_on {
            self.sync_disk();
        }
    }
}

impl Mapper for Fds {
    fn cpu_read(&mut self, addr: u16) -> u8 {
        match addr {
            0x6000..=0xDFFF => self.ram[addr as usize - 0x6000],
            0xE000..=0xFFFF => self.prg[addr as usize & 0x1FFF],
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x4020 => self.irq_reload = (self.irq_reload & 0xFF00) | val as u16,
            0x4021 => self.irq_reload = (self.irq_reload & 0x00FF) | (val as u16) << 8,
            0x4022 if self.disk_regs => {
                self.irq_repeat = val & 0x01 != 0;
                self.irq_enabled = val & 0x02 != 0;
                if self.irq_enabled {
                    self.irq_counter = self.irq_reload;
                } else {
                    self.timer_irq = false;
                }
            }
            0x4023 => {
                self.disk_regs = val & 0x01 != 0;
                self.sound_regs = val & 0x02 != 0;
                if !self.disk_regs {
                    self.irq_enabled = false;
                    self.timer_irq = false;
                    self.disk_irq = false;
                }
            }
            0x4024 if self.disk_regs => {
                self.write_data = val;
                self.transfer_complete = false;
                self.disk_irq = false;
            }
            0x4025 if self.disk_regs => self.write_control(val),
            0x4026 if self.disk_regs => self.ext_write = val,
            0x4040..=0x408A if self.sound_regs => self.audio.write(addr, val),
            0x6000..=0xDFFF => self.ram[addr as usize - 0x6000] = val,
            _ => {}
        }
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        Some(self.ram[(addr & 0x1FFF) as usize])
    }

    fn cpu_reg_read(&mut self, addr: u16) -> Option<u8> {
        match addr {
            0x4030 if self.disk_regs => {
                let mut v = 0;
                if self.timer_irq {
                    v |= 0x01;
                }
                if self.transfer_complete {
                    v |= 0x02;
                }
                if self.crc_error {
                    v |= 0x10;
                }
                if self.end_of_head {
                    v |= 0x40;
                }
                self.transfer_complete = false;
                self.timer_irq = false;
                self.disk_irq = false;
                Some(v)
            }
            0x4031 if self.disk_regs => {
                self.transfer_complete = false;
                self.disk_irq = false;
                Some(self.read_data)
            }
            0x4032 if self.disk_regs => {
                let inserted = self.side.is_some();
                let mut v = 0x40;
                if !inserted {
                    v |= 0x01 | 0x04; // no disk; not writable
                }
                if !inserted || !self.scanning {
                    v |= 0x02; // not ready
                }
                Some(v)
            }
            // Bit 7: battery good. Bits 0-6 echo the expansion port outputs.
            0x4033 if self.disk_regs => Some(0x80 | (self.ext_write & 0x7F)),
            0x4040..=0x4092 if self.sound_regs => self.audio.read(addr),
            _ => None,
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[addr as usize & 0x1FFF]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        self.chr[addr as usize & 0x1FFF] = val;
    }

    fn mirroring(&self) -> Mirroring {
        if self.horizontal {
            Mirroring::Horizontal
        } else {
            Mirroring::Vertical
        }
    }

    fn irq(&self) -> bool {
        self.timer_irq || self.disk_irq
    }

    fn cpu_clock(&mut self) {
        self.clock_timer();
        self.clock_drive();
        self.audio.clock();
    }

    fn audio_sample(&self) -> f32 {
        self.audio.sample()
    }

    /// The disk in `.fds` layout (all sides, no header), as last synced.
    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.disk)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.disk)
    }

    fn battery_ram_loaded(&mut self) {
        self.rebuild_streams();
    }

    fn fds_side_count(&self) -> Option<usize> {
        Some(self.streams.len())
    }

    fn fds_current_side(&self) -> Option<usize> {
        self.side
    }

    fn fds_bios_disk_check(&mut self, expected: &[u8; 10]) {
        if !self.auto_insert {
            return;
        }
        let matches: Vec<usize> = (0..self.streams.len())
            .filter(|&i| {
                let id = self
                    .disk
                    .get(i * SIDE_SIZE + DISK_ID..i * SIDE_SIZE + DISK_ID + DISK_ID_LEN);
                id.is_some_and(|id| expected.iter().zip(id).all(|(&e, &d)| e == 0xFF || e == d))
            })
            .collect();
        match matches.as_slice() {
            [] => {}
            [only] => {
                if self.side != Some(*only) {
                    // Swap straight in: the BIOS is waiting on this check.
                    self.sync_disk();
                    self.side = Some(*only);
                    self.pending_side = None;
                    self.insert_delay = 0;
                    self.scanning = false;
                    self.end_of_head = true;
                }
            }
            // Several sides share an ID (some unlicensed disks): guessing
            // would loop, so leave side changes to the player.
            _ => self.auto_insert = false,
        }
    }

    fn fds_set_auto_insert(&mut self, on: bool) {
        self.auto_insert = on;
    }

    fn fds_insert(&mut self, side: Option<usize>) {
        self.sync_disk();
        self.side = None;
        self.scanning = false;
        self.end_of_head = true;
        match side.filter(|&s| s < self.streams.len()) {
            Some(s) => {
                self.pending_side = Some(s);
                self.insert_delay = INSERT_DELAY;
            }
            None => {
                self.pending_side = None;
                self.insert_delay = 0;
            }
        }
    }

    fn save_state(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serialize FDS state")
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        let mut new: Fds = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        if new.streams.len() != self.streams.len() {
            return Err(format!(
                "savestate has {} disk sides; this disk has {}",
                new.streams.len(),
                self.streams.len()
            ));
        }
        if new.ram.len() != 0x8000 || new.chr.len() != 0x2000 {
            return Err("savestate FDS RAM sizes are wrong".into());
        }
        let sides = new.streams.len();
        if new.side.is_some_and(|s| s >= sides)
            || new.pending_side.is_some_and(|s| s >= sides)
            || new.dirty.is_some_and(|s| s >= sides)
        {
            return Err("savestate FDS drive state is out of range".into());
        }
        new.prg = std::mem::take(&mut self.prg);
        new.disk = new
            .streams
            .iter()
            .map(|s| stream_to_side(&s.0))
            .collect::<Vec<_>>()
            .concat();
        new.dirty = None;
        *self = new;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small but well-formed side: disk info, file count, one 5-byte file.
    pub(crate) fn sample_side() -> Vec<u8> {
        let mut s = vec![0x01];
        s.extend_from_slice(b"*NINTENDO-HVC*");
        s.resize(56, 0x11);
        s.extend_from_slice(&[0x02, 0x01]);
        let mut hdr = vec![0x03, 0x00, 0x00];
        hdr.extend_from_slice(b"FILENAME");
        hdr.extend_from_slice(&[0x00, 0x60, 0x05, 0x00, 0x00]); // $6000, 5 bytes, PRG
        s.extend_from_slice(&hdr);
        s.extend_from_slice(&[0x04, 1, 2, 3, 4, 5]);
        s.resize(SIDE_SIZE, 0);
        s
    }

    fn fds() -> Fds {
        Fds::new(vec![0xEA; 0x2000], vec![sample_side(), sample_side()])
    }

    #[test]
    fn parses_fwnes_and_raw_images() {
        let side = sample_side();
        let mut fwnes = b"FDS\x1A\x02".to_vec();
        fwnes.resize(HEADER_LEN, 0);
        fwnes.extend_from_slice(&side);
        fwnes.extend_from_slice(&side);
        let sides = parse_disk_image(&fwnes).unwrap();
        assert_eq!(sides.len(), 2);
        assert_eq!(sides[1], side);

        let raw = [side.clone(), side.clone(), side.clone()].concat();
        assert_eq!(parse_disk_image(&raw).unwrap().len(), 3);

        // A truncated raw dump pads out its last side.
        let short = parse_disk_image(&side[..1000]).unwrap();
        assert_eq!(short.len(), 1);
        assert_eq!(short[0].len(), SIDE_SIZE);
        assert_eq!(&short[0][..1000], &side[..1000]);

        assert!(parse_disk_image(b"NES\x1A0000000000000000").is_err());
    }

    #[test]
    fn gapped_stream_round_trips() {
        let side = sample_side();
        let stream = side_to_stream(&side);
        assert!(stream[..LEAD_GAP].iter().all(|&b| b == 0));
        assert_eq!(stream[LEAD_GAP], 0x80);
        assert_eq!(stream[LEAD_GAP + 1], 0x01);
        assert_eq!(stream_to_side(&stream), side);
    }

    #[test]
    fn block_crc_checks_out() {
        // Running the CRC over mark + block + stored CRC leaves zero, which
        // is what the drive's CRC check relies on.
        let stream = side_to_stream(&sample_side());
        let mut crc = 0;
        for &b in &stream[LEAD_GAP..LEAD_GAP + 1 + 56 + 2] {
            crc = crc_step(crc, b);
        }
        assert_eq!(crc, 0);
    }

    #[test]
    fn timer_irq_fires_after_reload_count_and_acks() {
        let mut m = fds();
        m.cpu_write(0x4023, 0x01);
        m.cpu_write(0x4020, 10);
        m.cpu_write(0x4021, 0);
        m.cpu_write(0x4022, 0x02); // enabled, one-shot
        for _ in 0..10 {
            m.clock_timer();
        }
        assert!(!m.irq());
        m.clock_timer();
        assert!(m.irq());
        assert_eq!(m.cpu_reg_read(0x4030).unwrap() & 1, 1);
        assert!(!m.irq(), "$4030 read acknowledges");
        for _ in 0..50 {
            m.clock_timer();
        }
        assert!(!m.irq(), "one-shot timer stops");
    }

    #[test]
    fn disabling_disk_io_gates_registers() {
        let mut m = fds();
        m.cpu_write(0x4023, 0x00);
        assert_eq!(m.cpu_reg_read(0x4032), None);
        m.cpu_write(0x4025, 0x08);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0x4023, 0x01);
        m.cpu_write(0x4025, 0x08);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn drive_reads_blocks_after_the_gap() {
        let mut m = fds();
        // Motor on, read mode, disk-ready ("CRC enabled") set; transfer
        // reset released.
        m.cpu_write(0x4025, 0x45);
        let mut got = Vec::new();
        for _ in 0..(REWIND_CYCLES as usize + (LEAD_GAP + 8) * (BYTE_CYCLES as usize + 1)) {
            m.cpu_clock();
            if m.transfer_complete {
                got.push(m.cpu_reg_read(0x4031).unwrap());
            }
        }
        assert_eq!(&got[..3], &[0x80, 0x01, b'*']);
        assert_eq!(m.cpu_reg_read(0x4032).unwrap() & 0x03, 0, "disk in, ready");
    }

    #[test]
    fn eject_and_insert_take_effect_after_a_delay() {
        let mut m = fds();
        assert_eq!(m.fds_side_count(), Some(2));
        assert_eq!(m.fds_current_side(), Some(0));
        m.fds_insert(Some(1));
        assert_eq!(m.fds_current_side(), None);
        assert_eq!(m.cpu_reg_read(0x4032).unwrap() & 1, 1);
        for _ in 0..INSERT_DELAY {
            m.clock_drive();
        }
        assert_eq!(m.fds_current_side(), Some(1));
        m.fds_insert(None);
        for _ in 0..INSERT_DELAY {
            m.clock_drive();
        }
        assert_eq!(m.fds_current_side(), None);
    }

    #[test]
    fn bios_disk_check_inserts_the_matching_side() {
        let mut a = sample_side();
        let mut b = sample_side();
        a[DISK_ID + 6] = 0; // side number byte differs
        b[DISK_ID + 6] = 1;
        let mut m = Fds::new(vec![0; 0x2000], vec![a.clone(), b.clone()]);
        assert_eq!(m.fds_current_side(), Some(0));
        let mut want = [0xFF; 10];
        want[6] = 1;
        m.fds_bios_disk_check(&want);
        assert_eq!(m.fds_current_side(), Some(1), "side B auto-inserted");
        // A wildcard ID matching both sides disables auto insertion.
        m.fds_bios_disk_check(&[0xFF; 10]);
        want[6] = 0;
        m.fds_bios_disk_check(&want);
        assert_eq!(m.fds_current_side(), Some(1), "auto insert now off");
    }

    #[test]
    fn crc_error_flag_only_for_corrupt_blocks() {
        let read_block = |side: Vec<u8>, corrupt: bool| -> bool {
            let mut stream = side_to_stream(&side);
            let mark = stream.iter().position(|&b| b != 0).unwrap();
            if corrupt {
                stream[mark + 5] ^= 0xFF;
            }
            let mut m = Fds::new(vec![0; 0x2000], vec![side]);
            m.streams[0].0 = stream;
            m.write_control(0x45 | 0x40); // motor, read, disk ready
            let mut error = false;
            let mut bytes = 0;
            for _ in 0..2_000_000 {
                m.clock_drive();
                if m.transfer_complete {
                    m.transfer_complete = false;
                    bytes += 1;
                    // Disk info block: 56 data bytes after the mark.
                    if bytes == 56 {
                        m.write_control(0x45 | 0x40 | 0x10); // CRC control
                    }
                    if bytes > 59 {
                        error = m.cpu_reg_read(0x4030).unwrap() & 0x10 != 0;
                        break;
                    }
                }
            }
            error
        };
        assert!(!read_block(sample_side(), false), "good block reads clean");
        assert!(
            read_block(sample_side(), true),
            "corrupt block flags CRC error"
        );
    }

    #[test]
    fn writes_fold_back_into_fds_layout() {
        let mut m = fds();
        // Overwrite the first file-data byte straight in the stream, as a
        // write session would, then end the session.
        let side = sample_side();
        let data_at = side.iter().position(|&b| b == 0x04).unwrap();
        let stream = side_to_stream(&side);
        // Find the block-4 mark: last $80 before the $04 byte.
        let pos = stream.windows(2).rposition(|w| w == [0x80, 0x04]).unwrap();
        m.streams[0].0[pos + 2] = 0xEE;
        m.dirty = Some(0);
        m.cpu_write(0x4025, 0x00); // motor off -> sync
        assert_eq!(m.prg_ram().unwrap()[data_at + 1], 0xEE);
        assert_eq!(
            m.prg_ram().unwrap()[SIDE_SIZE + data_at + 1],
            1,
            "side 2 untouched"
        );
    }

    #[test]
    fn drive_write_session_appends_a_block() {
        let mut m = fds();
        let side = sample_side();
        let raw_end = 56 + 2 + 16 + 6; // bytes used by the sample side
        let stream = side_to_stream(&side);
        let data_end = stream.iter().rposition(|&b| b != 0).unwrap();

        // Read until the head is in the trailing gap.
        m.cpu_write(0x4025, 0x45);
        while m.position < data_end + 40 {
            m.cpu_clock();
        }
        // Switch to write mode and feed gap, mark, a 16-byte file header.
        let mut header = vec![0x03, 0x01, 0x01];
        header.extend_from_slice(b"SAVEDATA");
        header.extend_from_slice(&[0x00, 0x60, 0x00, 0x00, 0x00]);
        let step = |m: &mut Fds| {
            let p = m.position;
            while m.position == p {
                m.cpu_clock();
            }
        };
        m.cpu_write(0x4025, 0x41);
        for b in [0, 0, 0, 0, 0x80].into_iter().chain(header.iter().copied()) {
            m.cpu_write(0x4024, b);
            step(&mut m);
        }
        m.cpu_write(0x4025, 0x51); // CRC control: drive emits the CRC
        step(&mut m);
        step(&mut m);
        m.cpu_write(0x4025, 0x41);
        m.cpu_write(0x4024, 0);
        step(&mut m);
        step(&mut m);
        m.cpu_write(0x4025, 0x45); // back to read mode -> fold back

        let disk = m.prg_ram().unwrap();
        assert_eq!(&disk[..raw_end], &side[..raw_end], "existing blocks intact");
        assert_eq!(&disk[raw_end..raw_end + 16], header.as_slice());

        // The written CRC validates: mark + block + CRC runs to zero.
        let s = &m.streams[0].0;
        let mark = s.iter().rposition(|&b| b == 0x80).unwrap();
        let crc = s[mark..mark + 1 + 16 + 2]
            .iter()
            .fold(0, |c, &b| crc_step(c, b));
        assert_eq!(crc, 0);
    }

    #[test]
    fn battery_restore_rebuilds_streams() {
        let mut m = fds();
        let mut blob = m.prg_ram().unwrap().to_vec();
        let data_at = blob.iter().position(|&b| b == 0x04).unwrap();
        blob[data_at + 1] = 0x99;
        m.prg_ram_mut().unwrap().copy_from_slice(&blob);
        m.battery_ram_loaded();
        assert_eq!(stream_to_side(&m.streams[0].0), blob[..SIDE_SIZE]);
    }

    #[test]
    fn savestate_round_trip() {
        let mut m = fds();
        m.cpu_write(0x6123, 0x42);
        m.cpu_write(0xD000, 0x43);
        m.ppu_write(0x0100, 0x44);
        m.cpu_write(0x4025, 0x4D);
        for _ in 0..1000 {
            m.cpu_clock();
        }
        m.streams[1].0[LEAD_GAP + 1 + 20] = 0x77; // a disk write
        m.dirty = Some(1);
        let blob = m.save_state();

        let mut r = Fds::new(vec![0xEA; 0x2000], vec![sample_side(), sample_side()]);
        r.load_state(&blob).unwrap();
        assert_eq!(r.cpu_read(0x6123), 0x42);
        assert_eq!(r.cpu_read(0xD000), 0x43);
        assert_eq!(r.ppu_read(0x0100), 0x44);
        assert_eq!(r.cpu_read(0xE000), 0xEA, "BIOS grafted from live instance");
        assert_eq!(r.mirroring(), Mirroring::Horizontal);
        assert_eq!(r.position, m.position);
        assert_eq!(r.prg_ram().unwrap()[SIDE_SIZE + 20], 0x77);

        let mut wrong = Fds::new(vec![0; 0x2000], vec![sample_side()]);
        assert!(wrong.load_state(&blob).is_err());
    }
}
