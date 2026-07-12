mod command;
mod config;
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

    loop {
        event_loop
            .dispatch(None, &mut state)
            .expect("Failed to dispatch events");
    }
}
