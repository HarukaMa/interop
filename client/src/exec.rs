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
    let (program, translated_args) = wrap_script_invocation(program, translated_args);

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
            Ok(_) => loop {
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
            },
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

/// Wrap Windows script extensions with their host shell.
///
/// - `.cmd` / `.bat` => `cmd.exe /d /s /c <script> ...`
/// - `.ps1` => `powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File <script> ...`
fn wrap_script_invocation(program: String, args: Vec<String>) -> (String, Vec<String>) {
    let lower = program.to_ascii_lowercase();
    if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        let mut wrapped_args = Vec::with_capacity(args.len() + 4);
        wrapped_args.push("/d".to_string());
        wrapped_args.push("/s".to_string());
        wrapped_args.push("/c".to_string());
        wrapped_args.push(program);
        wrapped_args.extend(args);
        ("cmd.exe".to_string(), wrapped_args)
    } else if lower.ends_with(".ps1") {
        let mut wrapped_args = Vec::with_capacity(args.len() + 6);
        wrapped_args.push("-NoLogo".to_string());
        wrapped_args.push("-NoProfile".to_string());
        wrapped_args.push("-ExecutionPolicy".to_string());
        wrapped_args.push("Bypass".to_string());
        wrapped_args.push("-File".to_string());
        wrapped_args.push(program);
        wrapped_args.extend(args);
        ("powershell.exe".to_string(), wrapped_args)
    } else {
        (program, args)
    }
}

#[cfg(test)]
mod tests {
    use super::wrap_script_invocation;

    #[test]
    fn wraps_cmd_script_with_cmd_exe() {
        let (program, args) = wrap_script_invocation(
            "C:\\Users\\me\\AppData\\Roaming\\npm\\codex.cmd".to_string(),
            vec!["--help".to_string()],
        );
        assert_eq!(program, "cmd.exe");
        assert_eq!(
            args,
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "C:\\Users\\me\\AppData\\Roaming\\npm\\codex.cmd".to_string(),
                "--help".to_string(),
            ]
        );
    }

    #[test]
    fn wraps_ps1_script_with_powershell() {
        let (program, args) = wrap_script_invocation(
            "C:\\Users\\me\\AppData\\Roaming\\npm\\codex.ps1".to_string(),
            vec!["--version".to_string()],
        );
        assert_eq!(program, "powershell.exe");
        assert_eq!(
            args,
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                "C:\\Users\\me\\AppData\\Roaming\\npm\\codex.ps1".to_string(),
                "--version".to_string(),
            ]
        );
    }

    #[test]
    fn leaves_non_script_program_unchanged() {
        let (program, args) =
            wrap_script_invocation("C:\\Windows\\System32\\notepad.exe".to_string(), vec![]);
        assert_eq!(program, "C:\\Windows\\System32\\notepad.exe");
        assert!(args.is_empty());
    }
}
