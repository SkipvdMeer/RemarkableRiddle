//! Raw Linux evdev input_event handling.
//!
//! On the reMarkable 2 (32-bit armv7, 32-bit time_t) a struct input_event is
//! 16 bytes: timeval (2 x u32), type (u16), code (u16), value (i32).
//! We parse and serialize it by hand so nothing here depends on kernel
//! headers or crate ABI assumptions.

pub const EVENT_SIZE: usize = 16;

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_ABS: u16 = 0x03;

pub const SYN_REPORT: u16 = 0;

pub const BTN_TOOL_PEN: u16 = 320;
pub const BTN_TOOL_RUBBER: u16 = 321;
pub const BTN_TOUCH: u16 = 330;
pub const BTN_STYLUS: u16 = 331;

pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;
pub const ABS_PRESSURE: u16 = 0x18;
pub const ABS_DISTANCE: u16 = 0x19;
pub const ABS_TILT_X: u16 = 0x1a;
pub const ABS_TILT_Y: u16 = 0x1b;

pub const ABS_MT_SLOT: u16 = 0x2f;
pub const ABS_MT_POSITION_X: u16 = 0x35;
pub const ABS_MT_POSITION_Y: u16 = 0x36;
pub const ABS_MT_TRACKING_ID: u16 = 0x39;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}

impl Event {
    pub fn new(type_: u16, code: u16, value: i32) -> Self {
        Event { type_, code, value }
    }

    pub fn parse(buf: &[u8]) -> Self {
        Event {
            type_: u16::from_le_bytes([buf[8], buf[9]]),
            code: u16::from_le_bytes([buf[10], buf[11]]),
            value: i32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
        }
    }

    /// Timestamps are left zero; the kernel stamps injected events itself.
    pub fn encode(&self) -> [u8; EVENT_SIZE] {
        let mut buf = [0u8; EVENT_SIZE];
        buf[8..10].copy_from_slice(&self.type_.to_le_bytes());
        buf[10..12].copy_from_slice(&self.code.to_le_bytes());
        buf[12..16].copy_from_slice(&self.value.to_le_bytes());
        buf
    }
}

pub fn describe(ev: &Event) -> String {
    let type_name = match ev.type_ {
        EV_SYN => "SYN",
        EV_KEY => "KEY",
        EV_ABS => "ABS",
        _ => "???",
    };
    let code_name = match (ev.type_, ev.code) {
        (EV_KEY, BTN_TOOL_PEN) => "BTN_TOOL_PEN".into(),
        (EV_KEY, BTN_TOOL_RUBBER) => "BTN_TOOL_RUBBER".into(),
        (EV_KEY, BTN_TOUCH) => "BTN_TOUCH".into(),
        (EV_KEY, BTN_STYLUS) => "BTN_STYLUS".into(),
        (EV_ABS, ABS_X) => "ABS_X".into(),
        (EV_ABS, ABS_Y) => "ABS_Y".into(),
        (EV_ABS, ABS_PRESSURE) => "ABS_PRESSURE".into(),
        (EV_ABS, ABS_DISTANCE) => "ABS_DISTANCE".into(),
        (EV_ABS, ABS_TILT_X) => "ABS_TILT_X".into(),
        (EV_ABS, ABS_TILT_Y) => "ABS_TILT_Y".into(),
        (EV_ABS, ABS_MT_SLOT) => "ABS_MT_SLOT".into(),
        (EV_ABS, ABS_MT_POSITION_X) => "ABS_MT_POSITION_X".into(),
        (EV_ABS, ABS_MT_POSITION_Y) => "ABS_MT_POSITION_Y".into(),
        (EV_ABS, ABS_MT_TRACKING_ID) => "ABS_MT_TRACKING_ID".into(),
        (EV_SYN, SYN_REPORT) => "SYN_REPORT".into(),
        (_, c) => format!("code_{c}"),
    };
    format!("{type_name} {code_name} = {}", ev.value)
}
