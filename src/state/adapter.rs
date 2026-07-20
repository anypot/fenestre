//! Protocol adapter for `WMState`.
//!
//! This module owns the boundary between pure core state and River protocol
//! calls. It applies deferred effects and performs proxy bookkeeping for
//! newly created windows, outputs, and seats.
use super::effects::Effect;
use super::output::OutputId;
use super::seat::SeatId;
use super::window::WindowId;
use super::wm::WMState;
use wayland_client::QueueHandle;

macro_rules! from_config_to_protocol {
    ($from:ty => $to:ty { $($from_v:ident => $to_v:ident),* $(,)? }) => {
        impl From<$from> for $to {
            fn from(v: $from) -> Self {
                match v {
                    $(
                        <$from>::$from_v => Self::$to_v,
                    )*
                }
            }
        }
    };
}

macro_rules! apply_bool_setting {
    ($device:expr, $qh:expr, $entry:ident.$field:ident, $enabled:path, $disabled:path, $setter:ident) => {
        if let Some(val) = $entry.$field {
            let state = if val { $enabled } else { $disabled };
            let _ = $device.$setter(state, $qh, ());
        }
    };
}

macro_rules! apply_enum_setting {
    ($device:expr, $qh:expr, $entry:ident.$field:ident, $setter:ident) => {
        if let Some(val) = $entry.$field {
            let val = val.into();
            let _ = $device.$setter(val, $qh, ());
        }
    };
}

impl WMState {
    /// Set the River window proxy on an already-created window entry.
    pub(super) fn set_window_proxy(
        &mut self,
        window_id: WindowId,
        river_window: crate::protocol::river::river_window_management_v1::client::river_window_v1::RiverWindowV1,
    ) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.river_window = Some(river_window);
        }
    }

    /// Set the River output proxy on an already-created output entry.
    pub(super) fn set_output_proxy(
        &mut self,
        output_id: OutputId,
        river_output: crate::protocol::river::river_window_management_v1::client::river_output_v1::RiverOutputV1,
    ) {
        if let Some(output) = self.outputs.get_mut(&output_id) {
            output.river_output = Some(river_output);
        }
    }

    /// Set the River seat proxy on an already-created seat entry.
    pub(super) fn set_seat_proxy(
        &mut self,
        seat_id: SeatId,
        river_seat: crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1,
    ) {
        if let Some(seat) = self.seats.get_mut(&seat_id) {
            seat.river_seat = Some(river_seat);
        }
    }

    /// Drain all pending effects and apply their protocol calls.
    pub(super) fn apply_effects(&mut self, _qh: &QueueHandle<Self>, effects: Vec<Effect>) {
        for effect in effects {
            self.apply_effect(_qh, effect);
        }
    }

    /// Remove an output: dispatch core event and clean the proxy index.
    ///
    /// Destroys the `river_layer_shell_output_v1` child proxy before dispatching
    /// the core removal, as required by the river_layer_shell_v1 protocol: the
    /// child is made inert by the `river_output_v1.removed` event and must be
    /// destroyed to complete destruction of the output (and before the owner
    /// `river_output` proxy itself is dropped by the caller). The child proxy is
    /// `take`n out of the entry first because the core event removes the entry
    /// from `self.outputs`.
    pub(super) fn remove_output(
        &mut self,
        output_id: OutputId,
        proxy: &crate::protocol::river::river_window_management_v1::client::river_output_v1::RiverOutputV1,
    ) {
        if let Some(output) = self.outputs.get_mut(&output_id)
            && let Some(layer_output) = output.river_layer_shell_output.take()
        {
            layer_output.destroy();
        }
        self.handle_event(super::events::Event::OutputRemoved { output_id });
        self.outputs_by_proxy.remove(proxy);
    }

    /// Remove a seat: dispatch core event and clean the proxy index.
    ///
    /// Mirrors [`remove_output`]: destroys the `river_layer_shell_seat_v1` child
    /// proxy before dispatching the core removal, since the child is made inert
    /// by the `river_seat_v1.removed` event and must be destroyed before the
    /// owner `river_seat` proxy is dropped by the caller.
    pub(super) fn remove_seat(
        &mut self,
        seat_id: SeatId,
        proxy: &crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1,
    ) {
        if let Some(seat) = self.seats.get_mut(&seat_id)
            && let Some(layer_seat) = seat.river_layer_shell_seat.take()
        {
            layer_seat.destroy();
        }
        self.handle_event(super::events::Event::SeatRemoved { seat_id });
        self.seats_by_proxy.remove(proxy);
    }

    /// Remove a window: dispatch core event and clean the proxy index.
    pub(super) fn remove_window(
        &mut self,
        window_id: WindowId,
        proxy: &crate::protocol::river::river_window_management_v1::client::river_window_v1::RiverWindowV1,
    ) {
        self.handle_event(super::events::Event::WindowClosed { window_id });
        self.windows_by_proxy.remove(proxy);
    }

    fn apply_effect(&mut self, qh: &QueueHandle<Self>, effect: Effect) {
        match effect {
            Effect::ProposeDimensions {
                window_id,
                width,
                height,
            } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.propose_dimensions(width, height);
                }
            }
            Effect::Fullscreen {
                window_id,
                output_id,
            } => {
                if let Some(output) = self.outputs.get(&output_id)
                    && let Some(river_output) = output.river_output.as_ref()
                    && let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.fullscreen(river_output);
                }
            }
            Effect::ExitFullscreen { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.exit_fullscreen();
                }
            }
            Effect::UseSsd { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.use_ssd();
                }
            }
            Effect::EnsureNode { window_id } => {
                if let Some(window) = self.windows.get_mut(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    window.node = Some(river_window.get_node(qh, ()));
                }
            }
            Effect::SetBorders {
                window_id,
                edges,
                width,
                r,
                g,
                b,
                a,
            } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    // NOTE: River's drawBorders in Window.zig uses a zero-size
                    // workaround for disabled edges instead of calling
                    // wlr_scene_node_setEnabled(false) directly. This avoids a
                    // Zig 0.16.0 ReleaseSafe optimizer bug where extern C fn
                    // calls to wlroots are elided when the field appears to be
                    // dead (the zig-pkg struct binding reads @field(node,"enabled")
                    // is never read through the Zig type before the next write).
                    let edges = crate::protocol::river::river_window_management_v1::client::river_window_v1::Edges::from_bits(edges).unwrap_or(crate::protocol::river::river_window_management_v1::client::river_window_v1::Edges::empty());
                    river_window.set_borders(edges, width, r, g, b, a);
                }
            }
            Effect::SetPosition { window_id, x, y } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(node) = window.node.as_ref()
                {
                    node.set_position(x, y);
                }
            }
            Effect::PlaceTop { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(node) = window.node.as_ref()
                {
                    node.place_top();
                }
            }
            Effect::Close { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.close();
                }
            }
            Effect::FocusWindow { window_id } => {
                // While a layer surface holds exclusive focus, River ignores WMState
                // focus changes, so don't emit them.
                if let Some(seat_id) = self.current_seat
                    && let Some(seat) = self.seats.get(&seat_id)
                    && seat.layer_shell_focus == super::seat::LayerShellFocus::Exclusive
                {
                    return;
                }
                if let Some(seat_id) = self.current_seat
                    && let Some(seat) = self.seats.get(&seat_id)
                    && let Some(window) = self.windows.get(&window_id)
                {
                    // Focus is a River call; make it here in the adapter
                    // rather than delegating to `Seat::focus_window`, so
                    // the core `Seat` stays free of protocol calls.
                    if let (Some(river_seat), Some(river_window)) =
                        (&seat.river_seat, &window.river_window)
                    {
                        river_seat.focus_window(river_window);
                    }
                }
            }
            Effect::SetLayerShellDefault { output_id } => {
                // Must be called during a manage sequence; `apply_manage`
                // produces this effect and the runtime applies it within
                // `ManageStart`. No-op if the output's layer-shell proxy is
                // not yet created (the effect is re-emitted on the next change).
                if let Some(output) = self.outputs.get(&output_id)
                    && let Some(layer_output) = output.river_layer_shell_output.as_ref()
                {
                    layer_output.set_default();
                }
            }
            Effect::StartPointerOp { seat_id } => {
                // Begin an interactive pointer operation. Sent from
                // `apply_manage`; River ignores it unless it is made inside a
                // manage sequence, which is exactly where this effect is applied.
                if let Some(seat) = self.seats.get(&seat_id)
                    && let Some(river_seat) = seat.river_seat.as_ref()
                {
                    river_seat.op_start_pointer();
                }
            }
            Effect::EndPointerOp { seat_id } => {
                // End the interactive pointer operation started by
                // `StartPointerOp`. River keeps the op alive until this is
                // processed, so it is emitted (with `StartPointerOp`) during a
                // manage sequence.
                if let Some(seat) = self.seats.get(&seat_id)
                    && let Some(river_seat) = seat.river_seat.as_ref()
                {
                    river_seat.op_end();
                }
            }
            Effect::InformResizeStart { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.inform_resize_start();
                }
            }
            Effect::InformResizeEnd { window_id } => {
                if let Some(window) = self.windows.get(&window_id)
                    && let Some(river_window) = window.river_window.as_ref()
                {
                    river_window.inform_resize_end();
                }
            }
        }
    }

    /// Ensure a `river_layer_shell_output_v1` child proxy exists for the given
    /// output, creating it on demand.
    ///
    /// This is the adapter-side bookkeeping that issues the River protocol call:
    /// it binds the layer-shell output proxy via `RiverLayerShellV1::get_output`
    /// against the output's underlying `river_output` proxy. It is a no-op when
    /// the layer-shell global is not yet bound, when the output has no underlying
    /// `river_output`, or when a layer-shell output proxy was already created, so
    /// it is safe to call repeatedly (e.g. once per output on global arrival and
    /// once per newly created output).
    pub(super) fn ensure_layer_shell_output(
        &mut self,
        output_id: OutputId,
        qh: &QueueHandle<Self>,
    ) {
        let (Some(shell), Some(output)) =
            (self.layer_shell.as_ref(), self.outputs.get_mut(&output_id))
        else {
            return;
        };
        if output.river_layer_shell_output.is_some() {
            return;
        }
        // Guard against double get_output
        let Some(river_output) = output.river_output.as_ref() else {
            return;
        };
        // Carry OutputId as dispatch user-data so `non_exclusive_area` events
        // can be routed back to the owning output.
        output.river_layer_shell_output = Some(shell.get_output(river_output, qh, output_id));
        // The proxy just became available: if it belongs to the currently
        // focused output, flag the default for re-emission on the next manage
        // (it may now be routable where it previously was not).
        if self.focused_output == Some(output_id) {
            self.layer_shell_default_dirty = true;
        }
    }

    /// Ensure a `river_layer_shell_seat_v1` child proxy exists for the given
    /// seat, creating it on demand.
    ///
    /// Mirrors [`ensure_layer_shell_output`]: issues the River protocol call
    /// `RiverLayerShellV1::get_seat` against the seat's underlying `river_seat`
    /// proxy. It is a no-op when the layer-shell global is not yet bound, when
    /// the seat has no underlying `river_seat`, or when a layer-shell seat proxy
    /// was already created, so it is safe to call repeatedly (e.g. once per seat
    /// on global arrival and once per newly created seat).
    pub(super) fn ensure_layer_shell_seat(&mut self, seat_id: SeatId, qh: &QueueHandle<Self>) {
        let (Some(shell), Some(seat)) = (self.layer_shell.as_ref(), self.seats.get_mut(&seat_id))
        else {
            return;
        };
        if seat.river_layer_shell_seat.is_some() {
            return;
        }
        let Some(river_seat) = seat.river_seat.as_ref() else {
            return;
        };
        seat.river_layer_shell_seat = Some(shell.get_seat(river_seat, qh, seat_id));
    }

    /// Remove an input device: clean up all indexes and destroy its proxy.
    pub(super) fn remove_input_device(
        &mut self,
        device_id: crate::state::input::DeviceId,
        proxy: &crate::protocol::river::river_input_management_v1::client::river_input_device_v1::RiverInputDeviceV1,
    ) {
        let name = self.input_devices.get(&device_id).map(|d| d.name.clone());
        self.libinput_devices.remove(&device_id);
        self.libinput_devices_by_proxy
            .retain(|_k, v| *v != device_id);
        self.xkb_keyboards.remove(&device_id);
        self.xkb_keyboards_by_proxy.retain(|_k, v| *v != device_id);
        self.input_devices.remove(&device_id);
        self.input_devices_by_proxy.remove(proxy);
        if let Some(name) = name {
            self.input_devices_by_name.remove(&name);
        }
        proxy.destroy();
    }

    pub(super) fn remove_libinput_device(
        &mut self,
        proxy: &crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::RiverLibinputDeviceV1,
    ) {
        if let Some(device_id) = self.libinput_devices_by_proxy.remove(proxy) {
            self.libinput_devices.remove(&device_id);
        }
        proxy.destroy();
    }

    pub(super) fn remove_xkb_keyboard(
        &mut self,
        proxy: &crate::protocol::river::river_xkb_config_v1::client::river_xkb_keyboard_v1::RiverXkbKeyboardV1,
    ) {
        if let Some(device_id) = self.xkb_keyboards_by_proxy.remove(proxy) {
            self.xkb_keyboards.remove(&device_id);
        }
        proxy.destroy();
    }

    pub(super) fn register_libinput_device(
        &mut self,
        device_id: crate::state::input::DeviceId,
        proxy: &crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::RiverLibinputDeviceV1,
    ) {
        self.libinput_devices.insert(
            device_id,
            crate::state::input::LibinputDeviceState {
                proxy: proxy.clone(),
            },
        );
        self.libinput_devices_by_proxy
            .insert(proxy.clone(), device_id);
    }

    pub(super) fn register_xkb_keyboard(
        &mut self,
        device_id: crate::state::input::DeviceId,
        proxy: &crate::protocol::river::river_xkb_config_v1::client::river_xkb_keyboard_v1::RiverXkbKeyboardV1,
    ) {
        self.xkb_keyboards.insert(
            device_id,
            crate::state::input::XkbKeyboardState {
                proxy: proxy.clone(),
                current_layout: 0,
            },
        );
        self.xkb_keyboards_by_proxy.insert(proxy.clone(), device_id);
    }

    /// Apply all device-related config: input devices, keyboard layout, repeat.
    pub(super) fn apply_device_config(&mut self, qh: &QueueHandle<Self>) {
        self.apply_input_config(qh);
        self.apply_keyboard_layout(qh);
        self.apply_repeat_config();
    }

    /// Apply configured input device settings to matching devices.
    pub(super) fn apply_input_config(&mut self, qh: &QueueHandle<Self>) {
        let Some(config) = self.config.as_ref() else {
            return;
        };
        let entries: Vec<crate::config::InputDeviceConfig> = config.input_devices.clone();
        for entry in entries {
            if let Some(device_id) = self.input_devices_by_name.get(&entry.name).copied() {
                self.apply_input_device_config(device_id, &entry, qh);
            } else {
                log::debug!(
                    target: "fenestre::state::adapter",
                    "Config entry for device {:?} did not match any known device",
                    entry.name,
                );
            }
        }
    }

    fn apply_input_device_config(
        &mut self,
        device_id: crate::state::input::DeviceId,
        entry: &crate::config::InputDeviceConfig,
        qh: &QueueHandle<Self>,
    ) {
        if !self.input_devices.contains_key(&device_id) {
            return;
        }
        let Some(lib_state) = self.libinput_devices.get(&device_id) else {
            return;
        };
        let device = &lib_state.proxy;
        let Some(libinput_config) = self.libinput_config.as_ref() else {
            return;
        };

        if let Some(profile) = entry.accel_profile {
            let profile_val = profile.into();
            let accel_config = libinput_config.create_accel_config(profile_val, qh, ());
            let _ = device.apply_accel_config(&accel_config, qh, ());
            accel_config.destroy();
        }

        if let Some(speed) = entry.accel_speed {
            let bytes = speed.to_ne_bytes().to_vec();
            let _ = device.set_accel_speed(bytes, qh, ());
        }

        if let Some(factor) = entry.scroll_factor
            && factor >= 0.0
        {
            self.input_devices[&device_id]
                .proxy
                .set_scroll_factor(factor);
        }

        apply_bool_setting!(
            device,
            qh,
            entry.tap,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::TapState::Enabled,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::TapState::Disabled,
            set_tap
        );

        apply_enum_setting!(device, qh, entry.tap_button_map, set_tap_button_map);

        apply_bool_setting!(
            device,
            qh,
            entry.natural_scroll,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::NaturalScrollState::Enabled,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::NaturalScrollState::Disabled,
            set_natural_scroll
        );

        apply_bool_setting!(
            device,
            qh,
            entry.left_handed,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::LeftHandedState::Enabled,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::LeftHandedState::Disabled,
            set_left_handed
        );

        apply_enum_setting!(device, qh, entry.scroll_method, set_scroll_method);

        apply_bool_setting!(
            device,
            qh,
            entry.middle_emulation,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::MiddleEmulationState::Enabled,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::MiddleEmulationState::Disabled,
            set_middle_emulation
        );

        apply_bool_setting!(
            device,
            qh,
            entry.dwt,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::DwtState::Enabled,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::DwtState::Disabled,
            set_dwt
        );

        apply_enum_setting!(device, qh, entry.send_events, set_send_events);

        apply_bool_setting!(
            device,
            qh,
            entry.drag,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::DragState::Enabled,
            crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::DragState::Disabled,
            set_drag
        );

        apply_enum_setting!(device, qh, entry.drag_lock, set_drag_lock);

        apply_enum_setting!(device, qh, entry.click_method, set_click_method);

        if let Some(rot) = entry.rotation {
            let _ = device.set_rotation(rot, qh, ());
        }
    }

    /// Generate and apply the configured keyboard layout keymap.
    pub(super) fn apply_keyboard_layout(&mut self, qh: &QueueHandle<Self>) {
        // Only one in-flight keymap at a time; the RiverInputDeviceV1 and
        // RiverLibinputDeviceV1 Done handlers both call this for the same
        // physical device. If a keymap is already in-flight, defer the new
        // layout so it is applied after the current one completes.
        let Some(config) = self.config.as_ref() else {
            return;
        };
        let Some(layout) = config.keyboard_layout.as_ref() else {
            return;
        };
        if self.pending_keymap.is_some() {
            self.pending_keymap_layout = Some(layout.clone());
            return;
        }
        let Some(xkb_config) = self.xkb_config.as_ref() else {
            return;
        };

        let keymap_str = match xkbcommon::xkb::Keymap::new_from_names(
            &xkbcommon::xkb::Context::new(0),
            layout.rules.as_deref().unwrap_or(""),
            layout.model.as_deref().unwrap_or(""),
            &layout.layout,
            layout.variant.as_deref().unwrap_or(""),
            layout.options.as_deref().map(|s| s.to_string()),
            xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) {
            Some(km) => km.get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1),
            None => {
                log::warn!(target: "fenestre::state::adapter", "Failed to compile xkb keymap");
                return;
            }
        };

        let name = std::ffi::CString::new("river-xkb-keymap").unwrap();
        // SAFETY: `name` is a valid NUL-terminated C string; `MFD_CLOEXEC` is a
        // valid flag. The returned fd is checked for `< 0` on the next line.
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            log::warn!(target: "fenestre::state::adapter", "memfd_create failed: {}", std::io::Error::last_os_error());
            return;
        }
        // SAFETY:
        // - `keymap_str` is a valid `String`; its pointer and length are stable
        //   for the duration of this block and it is not modified concurrently.
        // - `fd` is a valid, owned file descriptor returned by `memfd_create`
        //   on the previous line; it is not shared or used elsewhere.
        // - `libc::write` and `libc::lseek` are standard POSIX syscalls with
        //   well-defined behavior on a valid fd; errors are checked and fd is
        //   closed on failure.
        // - After `lseek` back to 0, the memfd contains the full keymap and
        //   is ready to be borrowed as a `BorrowedFd` for the Wayland call.
        unsafe {
            let ptr = keymap_str.as_ptr() as *const libc::c_void;
            let len = keymap_str.len();
            if libc::write(fd, ptr, len) < 0 {
                log::warn!(target: "fenestre::state::adapter", "write to memfd failed: {}", std::io::Error::last_os_error());
                libc::close(fd);
                return;
            }
            if libc::lseek(fd, 0, libc::SEEK_SET) < 0 {
                log::warn!(target: "fenestre::state::adapter", "lseek on memfd failed: {}", std::io::Error::last_os_error());
                libc::close(fd);
                return;
            }
        }

        use std::os::unix::io::{BorrowedFd, FromRawFd};
        // SAFETY: `fd` is a valid, owned file descriptor from `memfd_create`.
        // `BorrowedFd::borrow_raw` creates a borrowed reference; `fd` remains
        // valid and is closed explicitly after the Wayland call completes.
        let fd_borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        let keymap_obj = xkb_config.create_keymap(
            fd_borrowed,
            crate::protocol::river::river_xkb_config_v1::client::river_xkb_config_v1::KeymapFormat::TextV1,
            qh,
            (),
        );
        self.pending_keymap = Some(keymap_obj);
        self.pending_keymap_fd = Some(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(fd) });
    }

    /// Apply the configured repeat rate and delay to matching keyboard devices.
    ///
    /// The River protocol sends both rate and delay in a single request, so
    /// both fields must be present in the config entry for it to be applied.
    /// If only one is set a warning is logged and the entry is skipped.
    pub(super) fn apply_repeat_config(&mut self) {
        let Some(config) = self.config.as_ref() else {
            return;
        };
        for entry in &config.input_devices {
            if entry.repeat_rate.is_none() || entry.repeat_delay.is_none() {
                if entry.repeat_rate.is_some() || entry.repeat_delay.is_some() {
                    log::warn!(
                        target: "fenestre::state::adapter",
                        "Both repeat_rate and repeat_delay must be set for device {:?}; \
                         the River protocol sends both values in a single request",
                        entry.name,
                    );
                }
                continue;
            }
            let Some(device_id) = self.input_devices_by_name.get(&entry.name).copied() else {
                continue;
            };
            let Some(device_state) = self.input_devices.get(&device_id) else {
                continue;
            };
            if device_state.device_type != 0 {
                continue;
            }
            let rate = entry.repeat_rate.unwrap();
            let delay = entry.repeat_delay.unwrap();
            if rate < 0 || delay < 0 {
                continue;
            }
            device_state.proxy.set_repeat_info(rate, delay);
        }
    }
}

// `Custom` is intentionally unreachable because the config interface does not
// expose it yet (it requires `set_points` acceleration curve configuration).
// Use a manual `From` impl instead of `from_config_to_protocol!` so we can
// annotate the arm with `unreachable!()`.
impl From<crate::config::AccelProfile> for crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::AccelProfile {
    fn from(v: crate::config::AccelProfile) -> Self {
        match v {
            crate::config::AccelProfile::None => Self::None,
            crate::config::AccelProfile::Flat => Self::Flat,
            crate::config::AccelProfile::Adaptive => Self::Adaptive,
            crate::config::AccelProfile::Custom => unreachable!(),
        }
    }
}

from_config_to_protocol!(crate::config::TapButtonMap => crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::TapButtonMap {
    Lrm => Lrm, Lmr => Lmr,
});

from_config_to_protocol!(crate::config::ScrollMethod => crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::ScrollMethod {
    None => NoScroll, TwoFinger => TwoFinger, Edge => Edge, OnButtonDown => OnButtonDown,
});

from_config_to_protocol!(crate::config::DragLockState => crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::DragLockState {
    Disabled => Disabled, EnabledTimeout => EnabledTimeout, EnabledSticky => EnabledSticky,
});

from_config_to_protocol!(crate::config::ClickMethod => crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::ClickMethod {
    None => None, ButtonAreas => ButtonAreas, Clickfinger => Clickfinger,
});

from_config_to_protocol!(crate::config::SendEventsMode => crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::SendEventsModes {
    Enabled => Enabled, Disabled => Disabled, DisabledOnExternalMouse => DisabledOnExternalMouse,
});

#[cfg(test)]
mod tests {
    #[test]
    fn accel_profile_converts_to_protocol_variants() {
        use crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::AccelProfile as P;
        assert_eq!(P::from(crate::config::AccelProfile::None), P::None);
        assert_eq!(P::from(crate::config::AccelProfile::Flat), P::Flat);
        assert_eq!(P::from(crate::config::AccelProfile::Adaptive), P::Adaptive);
    }

    #[test]
    fn tap_button_map_converts_to_protocol_variants() {
        use crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::TapButtonMap as P;
        assert_eq!(P::from(crate::config::TapButtonMap::Lrm), P::Lrm);
        assert_eq!(P::from(crate::config::TapButtonMap::Lmr), P::Lmr);
    }

    #[test]
    fn scroll_method_converts_to_protocol_variants() {
        use crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::ScrollMethod as P;
        assert_eq!(P::from(crate::config::ScrollMethod::None), P::NoScroll);
        assert_eq!(
            P::from(crate::config::ScrollMethod::TwoFinger),
            P::TwoFinger
        );
        assert_eq!(P::from(crate::config::ScrollMethod::Edge), P::Edge);
        assert_eq!(
            P::from(crate::config::ScrollMethod::OnButtonDown),
            P::OnButtonDown
        );
    }

    #[test]
    fn drag_lock_state_converts_to_protocol_variants() {
        use crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::DragLockState as P;
        assert_eq!(P::from(crate::config::DragLockState::Disabled), P::Disabled);
        assert_eq!(
            P::from(crate::config::DragLockState::EnabledTimeout),
            P::EnabledTimeout
        );
        assert_eq!(
            P::from(crate::config::DragLockState::EnabledSticky),
            P::EnabledSticky
        );
    }

    #[test]
    fn click_method_converts_to_protocol_variants() {
        use crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::ClickMethod as P;
        assert_eq!(P::from(crate::config::ClickMethod::None), P::None);
        assert_eq!(
            P::from(crate::config::ClickMethod::ButtonAreas),
            P::ButtonAreas
        );
        assert_eq!(
            P::from(crate::config::ClickMethod::Clickfinger),
            P::Clickfinger
        );
    }

    #[test]
    fn send_events_mode_converts_to_protocol_variants() {
        use crate::protocol::river::river_libinput_config_v1::client::river_libinput_device_v1::SendEventsModes as P;
        assert_eq!(P::from(crate::config::SendEventsMode::Enabled), P::Enabled);
        assert_eq!(
            P::from(crate::config::SendEventsMode::Disabled),
            P::Disabled
        );
        assert_eq!(
            P::from(crate::config::SendEventsMode::DisabledOnExternalMouse),
            P::DisabledOnExternalMouse
        );
    }
}
