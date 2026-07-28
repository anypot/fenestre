use std::{
    env,
    io::{BufReader, Read, Write},
    os::unix::net::UnixStream,
    process,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let query = env::args().nth(1).unwrap_or_else(|| "all".into());
    let json = match query.as_str() {
        "windows" | "outputs" | "focused" | "all" => {
            format!(r#""{query}""#)
        }
        _ => {
            eprintln!("Usage: fnsctl <windows|outputs|focused|all>");
            process::exit(1);
        }
    };

    let sock = env::var("XDG_RUNTIME_DIR")
        .map(|d| format!("{d}/fenestre-ipc"))
        .unwrap_or_else(|_| "/tmp/fenestre-ipc".into());

    let mut conn = UnixStream::connect(&sock)?;
    writeln!(conn, "{json}")?;

    let mut buf = Vec::new();
    BufReader::new(conn).read_to_end(&mut buf)?;
    let output = String::from_utf8_lossy(&buf);
    print!("{output}");
    Ok(())
}
