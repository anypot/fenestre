//! Wayland dispatch handlers for `WMState`.
//!
//! This module is the boundary between River protocol events and Fenestre runtime state.
//! It handles registry discovery, River manage/render events,
//! window/output/seat lifecycle events, and River xkb binding events.
//!
//! Layout policy is intentionally not handled here.
//! The BSP tree layout engine should consume state changes and window metadata from this module.
use super::{
    keybindings::XkbBindingId,
    output::{Output, OutputId},
    seat::Seat,
    window::Window,
    wm::WMState,
};
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
                state.apply_manage(qh);
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
                state.apply_render(qh);
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
                // Resolve a valid focused output before computing the window's
                // target: if no real output exists yet, place the window in a
                // temporary orphan tree. `Event::Output` self-heals any orphan
                // trees by draining them into the first real output.
                state.ensure_focused_output();
                let target_output = state.focused_output.unwrap_or_else(|| {
                    let output_id = state.next_output_id();
                    state.focused_output = Some(output_id);
                    output_id
                });
                let mut window = Window::new(window_id, target_output);
                window.river_window = Some(id.clone());
                state.windows.insert(window_id, window);
                state.windows_by_proxy.insert(id, window_id);
                state.index_window_in_output(window_id, target_output);
                state
                    .ensure_tree_for_output(target_output)
                    .insert_window(window_id.0);
                state.push_focus(window_id);
                state.pending_focus = Some(window_id);

                state.request_manage_dirty();
            }
            Event::Output { id } => {
                let output_id = state.next_output_id();
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowManagerV1 event: Output created, internal OutputId={output_id:?}"
                );
                let mut output = Output::new(output_id);
                output.river_output = Some(id.clone());
                state.outputs.insert(output_id, output);
                state.outputs_by_proxy.insert(id, output_id);

                // Self-heal any orphaned output trees. Orphans can appear when
                // the last output is removed (its tree is intentionally kept so
                // windows survive), or when a window is created before any real
                // output exists. Drain those orphans into the new output's tree
                // now.
                let orphaned: Vec<OutputId> = state
                    .output_trees
                    .keys()
                    .filter(|id| !state.outputs.contains_key(id))
                    .copied()
                    .collect();
                for orphaned_id in orphaned {
                    if state.focused_output == Some(orphaned_id) {
                        state.focused_output = Some(output_id);
                    }
                    state.reassign_output(orphaned_id, output_id);
                }

                if state.focused_output.is_none() {
                    state.focused_output = Some(output_id);
                }
            }
            Event::Seat { id } => {
                // Store River's seat proxy before reconciling bindings.
                let seat_id = state.next_seat_id();
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowManagerV1 event: Seat created, internal SeatId={seat_id:?}"
                );
                let mut seat = Seat::new(seat_id);
                seat.river_seat = Some(id.clone());
                state.seats.insert(seat_id, seat);
                state.seats_by_proxy.insert(id, seat_id);

                // Set as current seat if first.
                if state.current_seat.is_none() {
                    state.current_seat = Some(seat_id);
                }

                // New seats may change which bindings should exist at runtime.
                state.reconcile_keybindings();
                state.request_manage_dirty();
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
                // Reconcile the layout tree and global focus pointers for the
                // closed window: removes it from its tree, the window map, and
                // the focus stack, then routes focus via the tree's preferred
                // new focus. See `WMState::close_window_focus_reconcile`.
                state.close_window_focus_reconcile(window_id);
                // Drop the now-stale proxy mapping and free the River object.
                state.windows_by_proxy.remove(proxy);
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
            }
            Event::Dimensions { width, height } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Dimensions updated to ({width}, {height})"
                );
                if let Some((_, window)) = state.find_window_mut_by_proxy(proxy) {
                    window.set_dimensions(width, height);
                }
            }
            Event::AppId { app_id } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: App ID updated to {app_id:?}"
                );
                state.apply_window_metadata(proxy, |window| {
                    window.app_id = app_id;
                });
            }
            Event::Title { title } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Title updated to {title:?}"
                );
                state.apply_window_metadata(proxy, |window| {
                    window.title = title;
                });
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
                if let Some((_, window)) = state.find_window_mut_by_proxy(proxy) {
                    window.decoration_hint = Some(hint.into());
                }
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
                let Some((window_id, tree)) = state.tree_for_window_proxy(proxy) else {
                    return;
                };
                if tree.toggle_fullscreen(window_id.0) {
                    state.request_manage_dirty();
                }
            }
            Event::ExitFullscreenRequested => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverWindowV1 event: Exit fullscreen requested"
                );
                let Some((window_id, tree)) = state.tree_for_window_proxy(proxy) else {
                    return;
                };
                if tree.toggle_fullscreen(window_id.0) {
                    state.request_manage_dirty();
                }
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
                if let Some((_, window)) = state.find_window_mut_by_proxy(proxy) {
                    window.pid = unreliable_pid;
                }
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
                // Reassign the removed output's windows onto a surviving output
                // (if any), then let `remove_output_by_proxy` own the
                // `focused_output` fallback so the policy lives in one place.
                let reassign_target = state.outputs.keys().find(|k| **k != output_id).copied();
                if let Some(to_id) = reassign_target {
                    state.reassign_output(output_id, to_id);
                }
                state.windows_by_output.remove(&output_id);
                let output_id = state.remove_output_by_proxy(proxy, reassign_target);
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
                if let Some((_, output)) = state.find_output_mut_by_proxy(proxy) {
                    output.set_position(x, y);
                    state.request_manage_dirty();
                }
            }
            Event::Dimensions { width, height } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverOutputV1 event: Dimensions updated to ({width}, {height})"
                );
                if let Some((_, output)) = state.find_output_mut_by_proxy(proxy) {
                    output.set_dimensions(width, height);
                    state.request_manage_dirty();
                }
                // The output rect now exists, so re-run rule evaluation for its
                // windows: those whose rules were deferred because the output
                // geometry wasn't ready when their metadata arrived can now apply.
                if let Some(output_id) = state.outputs_by_proxy.get(proxy).copied() {
                    let proxies: Vec<RiverWindowV1> = state
                        .windows_for_output(output_id)
                        .map(|set| {
                            set.iter()
                                .filter_map(|id| {
                                    state.windows.get(id).and_then(|w| w.river_window.clone())
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    for rw in &proxies {
                        state.evaluate_window_rules(rw);
                    }
                }
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
                // Remove the seat from runtime state, destroy its River proxy,
                // and rebuild keybindings so removed seats no longer receive bindings.
                debug!(target: "fenestre::state::handlers", "RiverSeatV1 event: Seat removed");
                if let Some(seat_id) = state.remove_seat_by_proxy(proxy) {
                    debug!(
                        target: "fenestre::state::handlers",
                        "RiverSeatV1 event: Seat removed, internal SeatId={seat_id:?}"
                    );
                    proxy.destroy();
                    state.reconcile_keybindings();
                    state.request_manage_dirty();
                }
            }
            Event::WlSeat { name } => {
                debug!(
                    target: "fenestre::state::handlers",
                    "RiverSeatV1 event: WlSeat created with name {name}"
                );
                if let Some((_, seat)) = state.find_seat_mut_by_proxy(proxy) {
                    seat.wl_seat_name = name;
                }
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
                if let Some((window_id, _)) = state.find_window_mut_by_proxy(&window)
                    && state.focused_window != Some(window_id)
                {
                    state.focus_window_id(window_id);
                }
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
                if let Some((_, seat)) = state.find_seat_mut_by_proxy(proxy) {
                    seat.pointer_position = Some((x, y));
                }
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
