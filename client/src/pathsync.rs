use bytes::BytesMut;
use interop_protocol::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Query the server for its PATH and print a shell-compatible export statement.
pub async fn run_path_sync() -> i32 {
    let config = Config::load();
    let addr = format!("{}:{}", config.host, config.port);

    let mut stream = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("interop-client: failed to connect to {addr}: {e}");
            return 1;
        }
    };

    // Send path query request
    let frame = path_query_request_frame();
    let mut encoded = BytesMut::new();
    encode_frame(&frame, &mut encoded);
    if let Err(e) = stream.write_all(&encoded).await {
        eprintln!("interop-client: send error: {e}");
        return 1;
    }

    // Read response
    let mut buf = BytesMut::with_capacity(8192);
    loop {
        match stream.read_buf(&mut buf).await {
            Ok(0) => break,
            Ok(_) => {
                if let Ok(Some(frame)) = decode_frame(&mut buf) {
                    match frame.frame_type {
                        FrameType::PathQueryResponse => {
                            let resp: PathQueryResponse =
                                match serde_json::from_slice(&frame.payload) {
                                    Ok(r) => r,
                                    Err(e) => {
                                        eprintln!("interop-client: invalid response: {e}");
                                        return 1;
                                    }
                                };

                            // Translate Windows paths to Linux, skip paths that
                            // are already Unix-style (server running in MSYS2)
                            let linux_dirs: Vec<String> = resp
                                .path_dirs
                                .iter()
                                .filter(|d| !d.is_empty())
                                .map(|d| {
                                    if d.len() >= 2 && d.as_bytes()[1] == b':' {
                                        // Looks like a Windows drive path (e.g. C:\...)
                                        windows_to_linux(d)
                                    } else {
                                        // Already a Unix path, use as-is
                                        d.clone()
                                    }
                                })
                                .collect();

                            let path_str = linux_dirs.join(":");
                            let current_path =
                                std::env::var("PATH").unwrap_or_default();
                            if current_path.is_empty() {
                                println!("export PATH='{path_str}'");
                            } else {
                                println!("export PATH='{current_path}:{path_str}'");
                            }
                            return 0;
                        }
                        FrameType::Error => {
                            let msg = String::from_utf8_lossy(&frame.payload);
                            eprintln!("interop-client: server error: {msg}");
                            return 1;
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                eprintln!("interop-client: read error: {e}");
                return 1;
            }
        }
    }

    eprintln!("interop-client: no response from server");
    1
}
