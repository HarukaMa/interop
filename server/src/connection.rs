use bytes::BytesMut;
use interop_protocol::*;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::process::spawn_process;

type Writer = Arc<Mutex<WriteHalf<TcpStream>>>;

/// Handle a single client connection.
pub async fn handle_connection(stream: TcpStream, peer: std::net::SocketAddr) {
    info!("New connection from {peer}");

    let (reader, writer) = tokio::io::split(stream);
    let writer: Writer = Arc::new(Mutex::new(writer));
    let mut reader = reader;
    let mut buf = BytesMut::with_capacity(8192);

    loop {
        match reader.read_buf(&mut buf).await {
            Ok(0) => {
                info!("Connection closed by {peer}");
                return;
            }
            Ok(_) => {}
            Err(e) => {
                error!("Read error from {peer}: {e}");
                return;
            }
        }

        match decode_frame(&mut buf) {
            Ok(Some(frame)) => match frame.frame_type {
                FrameType::ExecRequest => {
                    let req: ExecRequest = match serde_json::from_slice(&frame.payload) {
                        Ok(r) => r,
                        Err(e) => {
                            send_error(&writer, &format!("Invalid exec request: {e}")).await;
                            return;
                        }
                    };
                    info!(
                        "Exec: {} {:?} in {} (interactive={})",
                        req.program, req.args, req.cwd, req.interactive
                    );
                    if req.interactive {
                        #[cfg(windows)]
                        {
                            handle_exec_interactive(req, reader, buf, writer).await;
                            return;
                        }
                        #[cfg(not(windows))]
                        {
                            send_error(&writer, "Interactive mode requires Windows").await;
                            return;
                        }
                    } else {
                        handle_exec_pipe(req, reader, buf, writer).await;
                        return;
                    }
                }
                FrameType::PathQueryRequest => {
                    info!("PATH query from {peer}");
                    handle_path_query(&writer).await;
                    return;
                }
                other => {
                    warn!("Unexpected frame type {other:?} before exec/query");
                    send_error(&writer, "Expected ExecRequest or PathQueryRequest").await;
                    return;
                }
            },
            Ok(None) => {}
            Err(e) => {
                error!("Frame decode error: {e}");
                return;
            }
        }
    }
}

// =============================================================================
// Interactive mode: ConPTY
// =============================================================================

#[cfg(windows)]
async fn handle_exec_interactive(
    req: ExecRequest,
    reader: ReadHalf<TcpStream>,
    initial_buf: BytesMut,
    writer: Writer,
) {
    use crate::conpty::win::*;

    let conpty = match ConPtyProcess::spawn(
        &req.program,
        &req.args,
        &req.cwd,
        &req.env,
        req.term_width,
        req.term_height,
    ) {
        Ok(c) => c,
        Err(e) => {
            send_error(&writer, &format!("Failed to spawn ConPTY process: {e}")).await;
            return;
        }
    };

    // Store handles as usize for Send safety (HANDLE is *mut c_void on some targets)
    let pty_handle = conpty.pty_handle as usize;
    let process_handle = conpty.process_handle as usize;
    let thread_handle = conpty.thread_handle as usize;

    // ConPTY output → StdoutData frames to client (blocking thread + channel)
    let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    std::thread::spawn(move || read_pty_output(conpty.pty_output, output_tx));

    let w_output = writer.clone();
    let output_task = tokio::spawn(async move {
        while let Some(data) = output_rx.recv().await {
            let frame = stdout_data_frame(&data);
            let mut encoded = BytesMut::new();
            encode_frame(&frame, &mut encoded);
            let mut w = w_output.lock().await;
            if w.write_all(&encoded).await.is_err() {
                break;
            }
        }
    });

    // Client stdin → ConPTY input (blocking thread + channel)
    let (input_tx, input_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    std::thread::spawn(move || write_pty_input(conpty.pty_input, input_rx));

    // Network reader task: reads frames, forwards StdinData to input channel,
    // handles TerminalResize, returns true on TCP disconnect.
    let stdin_task = tokio::spawn(async move {
        let mut reader = reader;
        let mut buf = initial_buf;

        loop {
            while let Ok(Some(frame)) = decode_frame(&mut buf) {
                match frame.frame_type {
                    FrameType::StdinData => {
                        if input_tx.send(frame.payload).await.is_err() {
                            return false;
                        }
                    }
                    FrameType::StdinClose => return false,
                    FrameType::TerminalResize => {
                        if let Some((w, h)) = parse_terminal_resize(&frame.payload) {
                            resize_pty(pty_handle as _, w, h);
                        }
                    }
                    _ => {}
                }
            }

            match reader.read_buf(&mut buf).await {
                Ok(0) => return true,
                Ok(_) => {}
                Err(_) => return true,
            }
        }
    });

    // Wait for process exit in a blocking thread, signal via mpsc (can recv multiple times)
    let (exit_tx, mut exit_rx) = tokio::sync::mpsc::channel::<i32>(1);
    std::thread::spawn(move || {
        let code = wait_process(process_handle as _);
        let _ = exit_tx.blocking_send(code);
    });

    let stdin_abort = stdin_task.abort_handle();

    tokio::select! {
        // Process exited
        Some(exit_code) = exit_rx.recv() => {
            // Close the pseudo console first so the output pipe gets EOF
            close_pty(pty_handle as _);
            // Now output_task will drain remaining data and finish
            let _ = output_task.await;
            stdin_abort.abort();

            let frame = exit_code_frame(exit_code);
            let mut encoded = BytesMut::new();
            encode_frame(&frame, &mut encoded);
            let mut w = writer.lock().await;
            let _ = w.write_all(&encoded).await;
            let _ = w.shutdown().await;

            close_process_handles(process_handle as _, thread_handle as _);
            info!("Interactive process exited with code {exit_code}");
        }
        result = stdin_task => {
            let disconnected = result.unwrap_or(true);
            if disconnected {
                warn!("Client disconnected from interactive session, process continues");
                close_pty(pty_handle as _);
                close_process_handles(process_handle as _, thread_handle as _);
            } else {
                // StdinClose — wait for process to finish
                let exit_code = exit_rx.recv().await.unwrap_or(1);
                // Close pseudo console so output pipe gets EOF
                close_pty(pty_handle as _);
                let _ = output_task.await;

                let frame = exit_code_frame(exit_code);
                let mut encoded = BytesMut::new();
                encode_frame(&frame, &mut encoded);
                let mut w = writer.lock().await;
                let _ = w.write_all(&encoded).await;
                let _ = w.shutdown().await;

                close_process_handles(process_handle as _, thread_handle as _);
                info!("Interactive process exited with code {exit_code}");
            }
        }
    }
}

// =============================================================================
// Pipe mode (non-interactive)
// =============================================================================

async fn handle_exec_pipe(
    req: ExecRequest,
    reader: ReadHalf<TcpStream>,
    initial_buf: BytesMut,
    writer: Writer,
) {
    let mut child = match spawn_process(&req.program, &req.args, &req.cwd, &req.env) {
        Ok(c) => c,
        Err(e) => {
            send_error(&writer, &format!("Failed to spawn process: {e}")).await;
            return;
        }
    };

    let mut child_stdin = child.stdin.take().unwrap();
    let mut child_stdout = child.stdout.take().unwrap();
    let mut child_stderr = child.stderr.take().unwrap();

    let w_stdout = writer.clone();
    let stdout_task = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            match child_stdout.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let frame = stdout_data_frame(&buf[..n]);
                    let mut encoded = BytesMut::new();
                    encode_frame(&frame, &mut encoded);
                    let mut w = w_stdout.lock().await;
                    if w.write_all(&encoded).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let w_stderr = writer.clone();
    let stderr_task = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            match child_stderr.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let frame = stderr_data_frame(&buf[..n]);
                    let mut encoded = BytesMut::new();
                    encode_frame(&frame, &mut encoded);
                    let mut w = w_stderr.lock().await;
                    if w.write_all(&encoded).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let stdin_task = tokio::spawn(async move {
        let mut reader = reader;
        let mut initial_buf = initial_buf;

        while let Ok(Some(frame)) = decode_frame(&mut initial_buf) {
            match frame.frame_type {
                FrameType::StdinData => {
                    if child_stdin.write_all(&frame.payload).await.is_err() {
                        return false;
                    }
                }
                FrameType::StdinClose => {
                    drop(child_stdin);
                    return false;
                }
                _ => {}
            }
        }

        let mut net_buf = initial_buf;
        loop {
            match reader.read_buf(&mut net_buf).await {
                Ok(0) => return true,
                Ok(_) => {
                    while let Ok(Some(frame)) = decode_frame(&mut net_buf) {
                        match frame.frame_type {
                            FrameType::StdinData => {
                                if child_stdin.write_all(&frame.payload).await.is_err() {
                                    return false;
                                }
                            }
                            FrameType::StdinClose => {
                                drop(child_stdin);
                                return false;
                            }
                            _ => {}
                        }
                    }
                }
                Err(_) => return true,
            }
        }
    });

    let stdin_abort = stdin_task.abort_handle();

    tokio::select! {
        status = child.wait() => {
            let exit_code = match status {
                Ok(s) => s.code().unwrap_or(1),
                Err(e) => {
                    error!("Wait error: {e}");
                    1
                }
            };

            let _ = tokio::join!(stdout_task, stderr_task);
            stdin_abort.abort();

            let frame = exit_code_frame(exit_code);
            let mut encoded = BytesMut::new();
            encode_frame(&frame, &mut encoded);
            let mut w = writer.lock().await;
            let _ = w.write_all(&encoded).await;
            let _ = w.shutdown().await;

            info!("Process exited with code {exit_code}");
        }
        result = stdin_task => {
            let disconnected = result.unwrap_or(true);
            if disconnected {
                warn!("Client disconnected, child process will continue running");
                stdout_task.abort();
                stderr_task.abort();
                match child.wait().await {
                    Ok(s) => info!("Orphaned process exited with code {:?}", s.code()),
                    Err(e) => error!("Orphaned process wait error: {e}"),
                }
            } else {
                let exit_code = match child.wait().await {
                    Ok(s) => s.code().unwrap_or(1),
                    Err(e) => {
                        error!("Wait error: {e}");
                        1
                    }
                };

                let _ = tokio::join!(stdout_task, stderr_task);

                let frame = exit_code_frame(exit_code);
                let mut encoded = BytesMut::new();
                encode_frame(&frame, &mut encoded);
                let mut w = writer.lock().await;
                let _ = w.write_all(&encoded).await;
                let _ = w.shutdown().await;

                info!("Process exited with code {exit_code}");
            }
        }
    }
}

async fn handle_path_query(writer: &Writer) {
    let path_var = std::env::var("PATH").unwrap_or_default();
    let dirs: Vec<String> = std::env::split_paths(&path_var)
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    let resp = PathQueryResponse { path_dirs: dirs };
    let frame = path_query_response_frame(&resp);
    let mut encoded = BytesMut::new();
    encode_frame(&frame, &mut encoded);

    let mut w = writer.lock().await;
    let _ = w.write_all(&encoded).await;
    let _ = w.shutdown().await;
}

async fn send_error(writer: &Writer, msg: &str) {
    let frame = error_frame(msg);
    let mut encoded = BytesMut::new();
    encode_frame(&frame, &mut encoded);
    let mut w = writer.lock().await;
    let _ = w.write_all(&encoded).await;
    let _ = w.shutdown().await;
}
