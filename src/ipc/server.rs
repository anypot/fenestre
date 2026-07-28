use crate::ipc::protocol::{IpcQuery, IpcResponse};
use crate::state::WMState;
use calloop::PostAction;
use std::io::{BufRead, Write};
use std::os::unix::io::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;

pub(crate) struct IpcConn {
    stream: UnixStream,
    reader: std::io::BufReader<UnixStream>,
    buffer: String,
}

impl IpcConn {
    pub(crate) fn new(stream: UnixStream) -> std::io::Result<Self> {
        let reader = std::io::BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            buffer: String::new(),
        })
    }
}

impl AsFd for IpcConn {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.stream.as_fd()
    }
}

pub(crate) fn bind_listener() -> std::io::Result<std::os::unix::net::UnixListener> {
    let sock = std::env::var("XDG_RUNTIME_DIR")
        .map(|d| format!("{d}/fenestre-ipc"))
        .unwrap_or_else(|_| "/tmp/fenestre-ipc".into());
    if let Err(e) = std::fs::remove_file(&sock)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!(target: "fenestre::ipc", "failed to remove stale socket {sock:?}: {e}");
    }
    let listener = std::os::unix::net::UnixListener::bind(&sock)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn send_error(stream: &mut UnixStream, msg: &str) {
    let resp = IpcResponse::error(msg.to_string());
    let json = match serde_json::to_string_pretty(&resp) {
        Ok(j) => j + "\n",
        Err(e) => {
            log::warn!(target: "fenestre::ipc", "failed to serialize error response: {e}");
            let _ = stream.write_all(br#"{"ok":false,"error":"internal serialization error"}"#);
            return;
        }
    };
    if let Err(e) = stream.write_all(json.as_bytes()) {
        log::warn!(target: "fenestre::ipc", "failed to send response: {e}");
    }
}

/// Process one query from a client, then remove the connection.
///
/// Each connection handles exactly one JSON query and is then torn down.
/// Clients must reconnect for subsequent requests. This keeps the per-connection
/// state minimal and avoids tracking client lifetime across multiple read cycles.
pub(crate) fn handle_client(conn: &mut IpcConn, state: &WMState) -> std::io::Result<PostAction> {
    match conn.reader.read_line(&mut conn.buffer) {
        Ok(0) => Ok(PostAction::Remove),
        Ok(_) => {
            let line = conn
                .buffer
                .trim_end_matches(&['\n', '\r'] as &[_])
                .to_owned();
            conn.buffer.clear();
            let query: IpcQuery = match serde_json::from_str(&line) {
                Ok(q) => q,
                Err(e) => {
                    send_error(&mut conn.stream, &format!("parse error: {e}"));
                    return Ok(PostAction::Remove);
                }
            };
            let resp = crate::ipc::snapshot::handle_query(query, state);
            let json = match serde_json::to_string_pretty(&resp) {
                Ok(j) => j + "\n",
                Err(e) => {
                    send_error(
                        &mut conn.stream,
                        &format!("internal serialization error: {e}"),
                    );
                    return Ok(PostAction::Remove);
                }
            };
            if let Err(e) = conn.stream.write_all(json.as_bytes()) {
                log::warn!(target: "fenestre::ipc", "failed to send response: {e}");
            }
            Ok(PostAction::Remove)
        }
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(PostAction::Continue),
        Err(e) => {
            log::warn!(target: "fenestre::ipc", "client read error: {e}");
            Ok(PostAction::Remove)
        }
    }
}
