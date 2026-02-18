use bytes::BytesMut;
use interop_protocol::*;
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::rawmode;

/// Execute a program on the Windows host via the interop server.
/// Returns the exit code from the remote process.
pub async fn run_exec(program: String, args: Vec<String>) -> i32 {
    let config = Config::load();
    let addr = format!("{}:{}", config.host, config.port);

    let stream = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("interop-client: failed to connect to {addr}: {e}");
            return 127;
        }
    };

    let interactive = rawmode::is_tty();

    // Enable raw mode for interactive sessions (ConPTY speaks VT sequences)
    let _raw_guard = if interactive {
        rawmode::enable_raw_mode()
    } else {
        None
    };

    let (term_width, term_height) = if interactive {
        rawmode::terminal_size()
    } else {
        (80, 24)
    };

    let (mut reader, writer) = io::split(stream);
    let writer = Arc::new(Mutex::new(writer));

    let cwd = std::env::current_dir()
        .map(|p| linux_to_windows(p.to_str().unwrap_or("/")))
        .unwrap_or_default();

    let translated_args: Vec<String> = args.iter().map(|a| translate_arg(a)).collect();

    let program = if looks_like_path(&program) {
        linux_to_windows(&program)
    } else {
        program
    };

    let req = ExecRequest {
        program,
        args: translated_args,
        cwd,
        env: vec![],
        interactive,
        term_width,
        term_height,
    };

    // Send exec request
    let frame = exec_request_frame(&req);
    let mut encoded = BytesMut::new();
    encode_frame(&frame, &mut encoded);
    {
        let mut w = writer.lock().await;
        if let Err(e) = w.write_all(&encoded).await {
            eprintln!("interop-client: send error: {e}");
            return 127;
        }
    }

    // Forward local stdin to server
    let w_stdin = writer.clone();
    let stdin_task = tokio::spawn(async move {
        let mut stdin = io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf).await {
                Ok(0) => {
                    let frame = stdin_close_frame();
                    let mut encoded = BytesMut::new();
                    encode_frame(&frame, &mut encoded);
                    let mut w = w_stdin.lock().await;
                    let _ = w.write_all(&encoded).await;
                    break;
                }
                Ok(n) => {
                    let frame = stdin_data_frame(&buf[..n]);
                    let mut encoded = BytesMut::new();
                    encode_frame(&frame, &mut encoded);
                    let mut w = w_stdin.lock().await;
                    if w.write_all(&encoded).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // SIGWINCH handler for terminal resize (interactive mode only)
    #[cfg(unix)]
    let resize_task = if interactive {
        let w_resize = writer.clone();
        Some(tokio::spawn(async move {
            let mut sig =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
                    .expect("failed to register SIGWINCH");
            loop {
                sig.recv().await;
                let (w, h) = rawmode::terminal_size();
                let frame = terminal_resize_frame(w, h);
                let mut encoded = BytesMut::new();
                encode_frame(&frame, &mut encoded);
                let mut wr = w_resize.lock().await;
                if wr.write_all(&encoded).await.is_err() {
                    break;
                }
            }
        }))
    } else {
        None
    };

    // Read frames from server
    let mut net_buf = BytesMut::with_capacity(8192);
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let mut exit_code: i32 = 1;

    loop {
        match reader.read_buf(&mut net_buf).await {
            Ok(0) => break,
            Ok(_) => {
                loop {
                    match decode_frame(&mut net_buf) {
                        Ok(Some(frame)) => match frame.frame_type {
                            FrameType::StdoutData => {
                                let _ = stdout.write_all(&frame.payload).await;
                                let _ = stdout.flush().await;
                            }
                            FrameType::StderrData => {
                                let _ = stderr.write_all(&frame.payload).await;
                                let _ = stderr.flush().await;
                            }
                            FrameType::ExitCode => {
                                if frame.payload.len() == 4 {
                                    exit_code = i32::from_be_bytes([
                                        frame.payload[0],
                                        frame.payload[1],
                                        frame.payload[2],
                                        frame.payload[3],
                                    ]);
                                }
                                stdin_task.abort();
                                #[cfg(unix)]
                                if let Some(t) = resize_task {
                                    t.abort();
                                }
                                return exit_code;
                            }
                            FrameType::Error => {
                                let msg = String::from_utf8_lossy(&frame.payload);
                                eprintln!("interop-client: server error: {msg}");
                                stdin_task.abort();
                                #[cfg(unix)]
                                if let Some(t) = resize_task {
                                    t.abort();
                                }
                                return 127;
                            }
                            _ => {}
                        },
                        Ok(None) => break,
                        Err(e) => {
                            eprintln!("interop-client: decode error: {e}");
                            stdin_task.abort();
                            #[cfg(unix)]
                            if let Some(t) = resize_task {
                                t.abort();
                            }
                            return 127;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("interop-client: read error: {e}");
                break;
            }
        }
    }

    stdin_task.abort();
    #[cfg(unix)]
    if let Some(t) = resize_task {
        t.abort();
    }
    exit_code
}
