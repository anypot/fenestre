//! Wayland dispatch handlers for `WMState`.
//!
//! This module is the boundary between River protocol events and Fenestre runtime state.
//! It handles registry discovery, River manage/render events,
//! window/output/seat lifecycle events, and River xkb binding events.
//!
//! Layout policy is intentionally not handled here.
//! The BSP tree layout engine should consume state changes and window metadata from this module.
use super::{keybindings::XkbBindingId, wm::WMState};
use crate::protocol::river::river_window_management_v1::client::river_node_v1::RiverNodeV1;
use crate::protocol::river::river_window_management_v1::client::river_output_v1::RiverOutputV1;
use crate::protocol::river::river_window_management_v1::client::river_seat_v1::RiverSeatV1;
use crate::protocol::river::river_window_management_v1::client::river_window_manager_v1::EVT_OUTPUT_OPCODE;
use crate::protocol::river::river_window_management_v1::client::river_window_manager_v1::EVT_SEAT_OPCODE;
use crate::protocol::river::river_window_management_v1::client::river_window_manager_v1::EVT_WINDOW_OPCODE;
use crate::protocol::river::river_window_management_v1::client::river_window_manager_v1::RiverWindowManagerV1;
use crate::protocol::river::river_window_management_v1::client::river_window_v1::RiverWindowV1;
use crate::protocol::river::river_xkb_bindings_v1::client::river_xkb_binding_v1::{
    Event as XkbBindingEvent, RiverXkbBindingV1,
};
use crate::protocol::river::river_xkb_bindings_v1::client::river_xkb_bindings_v1::RiverXkbBindingsV1;
use log::debug;
use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_registry};

impl Dispatch<wl_registry::WlRegistry, ()> for WMState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "river_window_manager_v1" => {
                    debug!(target: "fenestre::state::handlers", "Found river_window_manager_v1");
                    let rwm: RiverWindowManagerV1 = registry.bind(name, version, qh, ());
                    state.wm = Some(rwm);
                    state.request_manage_dirty();
                }
                "river_xkb_bindings_v1" => {
                    debug!(target: "fenestre::state::handlers", "Found river_xkb_bindings_v1");
                    let xkb: RiverXkbBindingsV1 = registry.bind(name, version, qh, ());
                    state.xkb_bindings = Some(xkb);
                    // Config may be loaded or reloaded before the XKB global becomes available,
                    // so request a River manage sequence when it appears.
                    if state.xkb_bindings_dirty {
                        state.request_manage_dirty();
                    }
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<RiverWindowManagerV1, ()> for WMState {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowManagerV1,
        event: <RiverWindowManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use crate::protocol::river::river_window_management_v1::client::river_window_manager_v1::Event;
        match event {
            Event::Unavailable => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowManagerV1 event: Unavailable"
                );
            }
            Event::Finished => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowManagerV1 event: Finished"
                );
            }
            Event::ManageStart => {
                let effects = state.apply_manage(qh);
                state.apply_effects(qh, effects);
                if state.xkb_bindings_dirty {
                    // Destroy stale bindings before creating/enabling the desired set.
                    state.destroy_pending_keybindings();

                    if state.configure_keybindings(qh) {
                        state.xkb_bindings_dirty = false;
                    }
                }
                proxy.manage_finish();
            }
            Event::RenderStart => {
                let effects = state.apply_render();
                state.apply_effects(qh, effects);
                proxy.render_finish();
            }
            Event::SessionLocked => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowManagerV1 event: Session Locked"
                );
            }
            Event::SessionUnlocked => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowManagerV1 event: Session Unlocked"
                );
            }
            Event::Window { id } => {
                let window_id = state.next_window_id();
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowManagerV1 event: Window created, internal WindowId={window_id:?}"
                );
                state.ensure_focused_output();
                let target_output = state.focused_output.unwrap_or_else(|| {
                    let output_id = state.next_output_id();
                    state.focused_output = Some(output_id);
                    output_id
                });
                let event = super::events::Event::WindowCreated {
                    window_id,
                    target_output,
                };
                state.windows_by_proxy.insert(id.clone(), window_id);
                state.handle_event(event);
                state.set_window_proxy(window_id, id.clone());

                state.request_manage_dirty();
            }
            Event::Output { id } => {
                let output_id = state.next_output_id();
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowManagerV1 event: Output created, internal OutputId={output_id:?}"
                );
                let event = super::events::Event::OutputCreated { output_id };
                state.outputs_by_proxy.insert(id.clone(), output_id);
                state.handle_event(event);
                state.set_output_proxy(output_id, id.clone());
            }
            Event::Seat { id } => {
                let seat_id = state.next_seat_id();
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowManagerV1 event: Seat created, internal SeatId={seat_id:?}"
                );
                let event = super::events::Event::SeatCreated { seat_id };
                state.seats_by_proxy.insert(id.clone(), seat_id);
                state.handle_event(event);
                state.set_seat_proxy(seat_id, id.clone());
            }
        }
    }

    wayland_client::event_created_child!(WMState, RiverWindowManagerV1, [
        EVT_WINDOW_OPCODE => (RiverWindowV1, ()),
        EVT_OUTPUT_OPCODE => (RiverOutputV1, ()),
        EVT_SEAT_OPCODE => (RiverSeatV1, ()),
    ]);
}

impl Dispatch<RiverWindowV1, ()> for WMState {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowV1,
        event: <RiverWindowV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use crate::protocol::river::river_window_management_v1::client::river_window_v1::Event;
        match event {
            Event::Closed => {
                let Some(window_id) = state.windows_by_proxy.get(proxy).copied() else {
                    proxy.destroy();
                    state.request_manage_dirty();
                    return;
                };
                state.remove_window(window_id, proxy);
                state.request_manage_dirty();
                proxy.destroy();
            }
            Event::DimensionsHint {
                min_width,
                min_height,
                max_width,
                max_height,
            } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Dimensions hint updated to ({}, {}) - ({}, {})",
                    min_width,
                    min_height,
                    max_width,
                    max_height
                );
                let Some(window_id) = state.windows_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::DimensionsHint {
                    window_id,
                    min_w: min_width,
                    min_h: min_height,
                    max_w: max_width,
                    max_h: max_height,
                };
                state.handle_event(event);
            }
            Event::Dimensions { width, height } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Dimensions updated to ({width}, {height})"
                );
            }
            Event::AppId { app_id } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: App ID updated to {app_id:?}"
                );
                let Some(window_id) = state.windows_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::AppIdUpdated { window_id, app_id };
                state.handle_event(event);
            }
            Event::Title { title } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Title updated to {title:?}"
                );
                let Some(window_id) = state.windows_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::TitleUpdated { window_id, title };
                state.handle_event(event);
            }
            Event::Parent { parent } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Parent updated to {parent:?}"
                );
            }
            Event::DecorationHint { hint } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Decoration hint updated to {hint:?}"
                );
                let Some(window_id) = state.windows_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::DecorationHintUpdated {
                    window_id,
                    hint: hint.into(),
                };
                state.handle_event(event);
            }
            Event::PointerMoveRequested { seat } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Pointer move requested for seat {seat:?}"
                );
            }
            Event::PointerResizeRequested { seat, edges } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Pointer resize requested for seat {seat:?} with edges {edges:?}"
                );
            }
            Event::ShowWindowMenuRequested { x, y } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Show window menu requested at ({x}, {y})"
                );
            }
            Event::MaximizeRequested => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Maximize requested"
                );
            }
            Event::UnmaximizeRequested => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Unmaximize requested"
                );
            }
            Event::FullscreenRequested { output: _ } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Fullscreen requested"
                );
                let Some(window_id) = state.windows_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::FullscreenRequested { window_id };
                state.handle_event(event);
            }
            Event::ExitFullscreenRequested => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Exit fullscreen requested"
                );
                let Some(window_id) = state.windows_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::ExitFullscreenRequested { window_id };
                state.handle_event(event);
            }
            Event::MinimizeRequested => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Minimize requested"
                );
            }
            Event::UnreliablePid { unreliable_pid } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Unreliable PID updated to {unreliable_pid}"
                );
                let Some(window_id) = state.windows_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::PidUpdated {
                    window_id,
                    pid: unreliable_pid as u32,
                };
                state.handle_event(event);
            }
            Event::PresentationHint { hint } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Presentation hint updated to {hint:?}"
                );
            }
            Event::Identifier { identifier } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Identifier updated to {identifier}"
                );
            }
            Event::CaptureSessions { count } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Capture sessions updated to {count}"
                );
            }
        }
    }
}

impl Dispatch<RiverNodeV1, ()> for WMState {
    fn event(
        _state: &mut Self,
        _proxy: &RiverNodeV1,
        _event: <RiverNodeV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<RiverOutputV1, ()> for WMState {
    fn event(
        state: &mut Self,
        proxy: &RiverOutputV1,
        event: <RiverOutputV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use crate::protocol::river::river_window_management_v1::client::river_output_v1::Event;
        match event {
            Event::Removed => {
                let Some(output_id) = state.outputs_by_proxy.get(proxy).copied() else {
                    proxy.destroy();
                    state.request_manage_dirty();
                    return;
                };
                state.remove_output(output_id, proxy);
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverOutputV1 event: Output removed, internal OutputId={output_id:?}"
                );
                proxy.destroy();
            }
            Event::WlOutput { name } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverOutputV1 event: WlOutput created with name {name}"
                );
            }
            Event::Position { x, y } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverOutputV1 event: Position updated to ({x}, {y})"
                );
                let Some(output_id) = state.outputs_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::OutputPositionUpdated { output_id, x, y };
                state.handle_event(event);
            }
            Event::Dimensions { width, height } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverOutputV1 event: Dimensions updated to ({width}, {height})"
                );
                let Some(output_id) = state.outputs_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::OutputDimensionsUpdated {
                    output_id,
                    w: width,
                    h: height,
                };
                state.handle_event(event);
            }
            Event::CaptureSessions { count } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverOutputV1 event: Capture sessions updated to {count}"
                );
            }
        }
    }
}

impl Dispatch<RiverSeatV1, ()> for WMState {
    fn event(
        state: &mut Self,
        proxy: &RiverSeatV1,
        event: <RiverSeatV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use crate::protocol::river::river_window_management_v1::client::river_seat_v1::Event;
        match event {
            Event::Removed => {
                debug!(target: "fenestre::state::handlers", "RiverSeatV1 event: Seat removed");
                let Some(seat_id) = state.seats_by_proxy.get(proxy).copied() else {
                    proxy.destroy();
                    return;
                };
                state.remove_seat(seat_id, proxy);
                proxy.destroy();
            }
            Event::WlSeat { name } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverSeatV1 event: WlSeat created with name {name}"
                );
                let Some(seat_id) = state.seats_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::SeatNameUpdated { seat_id, name };
                state.handle_event(event);
            }
            Event::PointerEnter { window } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverSeatV1 event: Pointer entered window {window:?}"
                );
            }
            Event::PointerLeave => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverSeatV1 event: Pointer left window"
                );
            }
            Event::WindowInteraction { window } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverSeatV1 event: Pointer interacted with window {window:?}"
                );
                let Some(window_id) = state.windows_by_proxy.get(&window).copied() else {
                    return;
                };
                let event = super::events::Event::WindowInteraction { window_id };
                state.handle_event(event);
            }
            Event::ShellSurfaceInteraction { shell_surface } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverSeatV1 event: Pointer interacted with shell surface {shell_surface:?}"
                );
            }
            Event::OpDelta { dx, dy } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverSeatV1 event: Pointer moved by dx {dx} and dy {dy})"
                );
            }
            Event::OpRelease => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverSeatV1 event: Pointer operation input released"
                );
            }
            Event::PointerPosition { x, y } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverSeatV1 event: Pointer position updated to ({x}, {y})"
                );
                let Some(seat_id) = state.seats_by_proxy.get(proxy).copied() else {
                    return;
                };
                let event = super::events::Event::SeatPointerPositionUpdated { seat_id, x, y };
                state.handle_event(event);
            }
        }
    }
}

impl Dispatch<RiverXkbBindingsV1, ()> for WMState {
    fn event(
        _state: &mut Self,
        _proxy: &RiverXkbBindingsV1,
        _event: <RiverXkbBindingsV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<RiverXkbBindingV1, XkbBindingId> for WMState {
    fn event(
        state: &mut Self,
        _proxy: &RiverXkbBindingV1,
        event: <RiverXkbBindingV1 as wayland_client::Proxy>::Event,
        data: &XkbBindingId,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            XkbBindingEvent::Pressed => {
                let Some(binding) = state.keybindings.get(data) else {
                    debug!(
                        target: "fenestre::state::handlers",
                        "Pressed unknown xkb binding id={data:?}"
                    );
                    return;
                };

                debug!(
                    target: "fenestre::state::handlers",
                    "Pressed xkb binding id={:?} seat_id={:?} keysym=0x{:x} modifiers=0x{:x} command={:?}",
                    data,
                    binding.seat_id,
                    binding.keysym,
                    binding.modifiers,
                    binding.command
                );

                let command = binding.command.clone();

                state.run_command(command, qh);
            }
            XkbBindingEvent::Released => {}
            XkbBindingEvent::StopRepeat => {}
        }
    }
}
