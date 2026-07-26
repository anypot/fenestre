//! Input device state types for River input management protocols.
//!
//! Owns the bookkeeping for `river_input_device_v1`, `river_libinput_device_v1`,
//! and `river_xkb_keyboard_v1` objects: proxy-to-ID indexes, per-device state,
//! and the lifecycle cleanup helpers consumed by `handlers.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct DeviceId(pub u32);

pub(super) struct InputDeviceState {
    pub proxy: crate::protocol::river::river_input_management_v1::client::river_input_device_v1::RiverInputDeviceV1,
    pub name: String,
    pub device_type: u32,
}

pub(super) struct LibinputDeviceState {
    pub proxy: crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::RiverLibinputDeviceV1,
}

pub(super) struct XkbKeyboardState {
    pub proxy: crate::protocol::river::river_xkb_config_v1::client::river_xkb_keyboard_v1::RiverXkbKeyboardV1,
    pub current_layout: u32,
}
