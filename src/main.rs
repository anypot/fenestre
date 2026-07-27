mod command;
mod config;
mod ipc;
mod layout;
mod protocol;
mod state;

use std::path::Path;

use calloop::EventLoop;
use calloop_wayland_source::WaylandSource;
use config::Config;
use log::{debug, info, warn};
use state::WMState;
use wayland_client::Connection;

fn log_config_load_error(err: impl std::fmt::Display, path: impl std::fmt::Display) {
    let msg = format!("Failed to load config from {path}: {err}; using built-in defaults");
    warn!(target: "fenestre::config", "{msg}");
    eprintln!("fenestre: {msg}");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let mut state = WMState::new();

    // Config load failures are non-fatal: `WMState::new` has already loaded the
    // built-in defaults, so on any error we log a warning and keep running with
    // those defaults instead of aborting startup. We also write directly to
    // stderr so the failure is visible regardless of the active log filter.
    if let Some(config_path) = std::env::args().nth(1) {
        info!(
            target: "fenestre::config",
            "Loading config from CLI argument: {}",
            config_path
        );
        if let Err(err) = state.load_config_file(Path::new(&config_path)) {
            log_config_load_error(err, config_path);
        }
    } else if let Some(config_path) = Config::default_path() {
        info!(
            target: "fenestre::config",
            "Loading default config: {}",
            config_path.display()
        );
        if let Err(err) = state.load_config_file(&config_path) {
            log_config_load_error(err, config_path.display());
        }
    } else {
        debug!(
            target: "fenestre::config",
            "No config path supplied and no default config found; using built-in defaults"
        );
    }

    let conn = Connection::connect_to_env().expect("Failed to connect to Wayland!");
    let display = conn.display();
    let event_queue = conn.new_event_queue();
    let qh = event_queue.handle().clone();

    let mut event_loop: EventLoop<WMState> =
        EventLoop::try_new().expect("Failed to initialize the event loop!");

    let handle = event_loop.handle();
    let wayland_source = WaylandSource::new(conn, event_queue);
    wayland_source
        .insert(handle)
        .expect("Failed to insert Wayland source into event loop");

    let _registry = display.get_registry(&qh, ());

    let inner_handle = event_loop.handle();
    let closure_handle = inner_handle.clone();
    let listener =
        ipc::server::bind_listener().map_err(|e| format!("Failed to bind IPC socket: {e}"))?;
    inner_handle
        .insert_source(
            calloop::generic::Generic::new(listener, calloop::Interest::READ, calloop::Mode::Level),
            move |_readiness, meta, _state: &mut WMState| {
                let listener = unsafe { meta.get_mut() };
                loop {
                    match listener.accept() {
                        Ok((stream, _addr)) => {
                            let conn = match ipc::server::IpcConn::new(stream) {
                                Ok(c) => c,
                                Err(e) => {
                                    log::warn!(target: "fenestre::ipc", "failed to wrap client: {e}");
                                    continue;
                                }
                            };
                            if let Err(e) = closure_handle.insert_source(
                                calloop::generic::Generic::new(
                                    conn,
                                    calloop::Interest::READ,
                                    calloop::Mode::Level,
                                ),
                                |_readiness, meta, state: &mut WMState| {
                                    let conn = unsafe { meta.get_mut() };
                                    ipc::server::handle_client(conn, state)
                                },
                            ) {
                                log::warn!(target: "fenestre::ipc", "failed to register client: {e}");
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) => {
                            log::warn!(target: "fenestre::ipc", "accept error: {e}");
                            break;
                        }
                    }
                }
                Ok(calloop::PostAction::Continue)
            },
        )
        .map_err(|e| format!("Failed to insert IPC source into event loop: {e}"))?;

    loop {
        event_loop
            .dispatch(None, &mut state)
            .expect("Failed to dispatch events");
    }
}
