/// ConPTY (Windows Pseudo Console) process spawning.
///
/// Creates a pseudo console that translates between VT100 escape sequences
/// on pipes and Windows Console API calls, enabling full interactive terminal
/// support for remote execution.
#[cfg(windows)]
pub mod win {
    use std::io::{self, Read, Write};
    use std::os::windows::io::FromRawHandle;
    use std::ptr;

    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::System::Console::*;
    use windows_sys::Win32::System::Pipes::*;
    use windows_sys::Win32::System::Threading::*;

    // Not always exported by windows-sys
    const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x00020016;
    const PSEUDOCONSOLE_INHERIT_CURSOR: u32 = 1;

    /// Result of spawning a ConPTY process. Each field is consumed separately.
    pub struct ConPtyProcess {
        /// Pseudo console handle — used for resize and must be closed on cleanup.
        pub pty_handle: HPCON,
        /// Process handle — used for WaitForSingleObject + GetExitCodeProcess.
        pub process_handle: HANDLE,
        /// Thread handle — closed on drop.
        pub thread_handle: HANDLE,
        /// Write end of the ConPTY input pipe.
        pub pty_input: std::fs::File,
        /// Read end of the ConPTY output pipe.
        pub pty_output: std::fs::File,
    }

    // SAFETY: HANDLEs are thread-safe on Windows.
    unsafe impl Send for ConPtyProcess {}

    impl ConPtyProcess {
        pub fn spawn(
            program: &str,
            args: &[String],
            cwd: &str,
            env: &[(String, String)],
            width: u16,
            height: u16,
        ) -> io::Result<Self> {
            unsafe {
                let mut pty_input_read: HANDLE = INVALID_HANDLE_VALUE;
                let mut pty_input_write: HANDLE = INVALID_HANDLE_VALUE;
                let mut pty_output_read: HANDLE = INVALID_HANDLE_VALUE;
                let mut pty_output_write: HANDLE = INVALID_HANDLE_VALUE;

                if CreatePipe(&mut pty_input_read, &mut pty_input_write, ptr::null(), 0) == 0 {
                    return Err(io::Error::last_os_error());
                }
                if CreatePipe(&mut pty_output_read, &mut pty_output_write, ptr::null(), 0) == 0 {
                    CloseHandle(pty_input_read);
                    CloseHandle(pty_input_write);
                    return Err(io::Error::last_os_error());
                }

                let size = COORD {
                    X: width as i16,
                    Y: height as i16,
                };
                let mut pty_handle: HPCON = 0;
                let hr = CreatePseudoConsole(
                    size,
                    pty_input_read,
                    pty_output_write,
                    PSEUDOCONSOLE_INHERIT_CURSOR,
                    &mut pty_handle,
                );

                // ConPTY now owns these ends
                CloseHandle(pty_input_read);
                CloseHandle(pty_output_write);

                if hr != 0 {
                    CloseHandle(pty_input_write);
                    CloseHandle(pty_output_read);
                    return Err(io::Error::from_raw_os_error(hr));
                }

                // Set up process attribute list with ConPTY
                let mut attr_size: usize = 0;
                InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attr_size);

                let attr_buf = vec![0u8; attr_size];
                let attr_list = attr_buf.as_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;

                if InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size) == 0 {
                    ClosePseudoConsole(pty_handle);
                    CloseHandle(pty_input_write);
                    CloseHandle(pty_output_read);
                    return Err(io::Error::last_os_error());
                }

                if UpdateProcThreadAttribute(
                    attr_list,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                    pty_handle as *const core::ffi::c_void,
                    size_of::<HPCON>(),
                    ptr::null_mut(),
                    ptr::null(),
                ) == 0
                {
                    DeleteProcThreadAttributeList(attr_list);
                    ClosePseudoConsole(pty_handle);
                    CloseHandle(pty_input_write);
                    CloseHandle(pty_output_read);
                    return Err(io::Error::last_os_error());
                }

                let cmdline = build_cmdline(program, args);
                let mut cmdline_wide: Vec<u16> =
                    cmdline.encode_utf16().chain(std::iter::once(0)).collect();

                let cwd_wide: Option<Vec<u16>> = if cwd.is_empty() {
                    None
                } else {
                    Some(cwd.encode_utf16().chain(std::iter::once(0)).collect())
                };

                let env_block: Option<Vec<u16>> = if env.is_empty() {
                    None
                } else {
                    let mut block = Vec::new();
                    for (k, v) in env {
                        let entry = format!("{k}={v}");
                        block.extend(entry.encode_utf16());
                        block.push(0);
                    }
                    block.push(0);
                    Some(block)
                };

                let mut startup_info: STARTUPINFOEXW = std::mem::zeroed();
                startup_info.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
                startup_info.lpAttributeList = attr_list;

                let mut proc_info: PROCESS_INFORMATION = std::mem::zeroed();

                let create_flags = EXTENDED_STARTUPINFO_PRESENT
                    | if env_block.is_some() {
                        CREATE_UNICODE_ENVIRONMENT
                    } else {
                        0
                    };

                let result = CreateProcessW(
                    ptr::null(),
                    cmdline_wide.as_mut_ptr(),
                    ptr::null(),
                    ptr::null(),
                    0,
                    create_flags,
                    env_block
                        .as_ref()
                        .map_or(ptr::null(), |b| b.as_ptr() as *const core::ffi::c_void),
                    cwd_wide.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
                    &startup_info.StartupInfo,
                    &mut proc_info,
                );

                DeleteProcThreadAttributeList(attr_list);

                if result == 0 {
                    ClosePseudoConsole(pty_handle);
                    CloseHandle(pty_input_write);
                    CloseHandle(pty_output_read);
                    return Err(io::Error::last_os_error());
                }

                let pty_input = std::fs::File::from_raw_handle(pty_input_write as *mut _);
                let pty_output = std::fs::File::from_raw_handle(pty_output_read as *mut _);

                Ok(ConPtyProcess {
                    pty_handle,
                    process_handle: proc_info.hProcess,
                    thread_handle: proc_info.hThread,
                    pty_input,
                    pty_output,
                })
            }
        }
    }

    /// Resize a pseudo console. Thread-safe.
    pub fn resize_pty(pty_handle: HPCON, width: u16, height: u16) {
        let size = COORD {
            X: width as i16,
            Y: height as i16,
        };
        unsafe {
            ResizePseudoConsole(pty_handle, size);
        }
    }

    /// Wait for process exit and return exit code. Blocking.
    pub fn wait_process(process_handle: HANDLE) -> i32 {
        unsafe {
            WaitForSingleObject(process_handle, INFINITE);
            let mut exit_code: u32 = 0;
            GetExitCodeProcess(process_handle, &mut exit_code);
            exit_code as i32
        }
    }

    /// Close just the pseudo console. This causes the output pipe to EOF.
    pub fn close_pty(pty_handle: HPCON) {
        unsafe {
            ClosePseudoConsole(pty_handle);
        }
    }

    /// Close process and thread handles.
    pub fn close_process_handles(process_handle: HANDLE, thread_handle: HANDLE) {
        unsafe {
            CloseHandle(process_handle);
            CloseHandle(thread_handle);
        }
    }

    /// Read from ConPTY output pipe into a channel. Blocking.
    pub fn read_pty_output(mut output: std::fs::File, sender: tokio::sync::mpsc::Sender<Vec<u8>>) {
        let mut buf = [0u8; 4096];
        loop {
            match output.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if sender.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    /// Write to ConPTY input pipe from a channel. Blocking.
    pub fn write_pty_input(
        mut input: std::fs::File,
        mut receiver: tokio::sync::mpsc::Receiver<Vec<u8>>,
    ) {
        while let Some(data) = receiver.blocking_recv() {
            if input.write_all(&data).is_err() {
                break;
            }
        }
    }

    fn build_cmdline(program: &str, args: &[String]) -> String {
        let mut cmd = quote_arg(program);
        for arg in args {
            cmd.push(' ');
            cmd.push_str(&quote_arg(arg));
        }
        cmd
    }

    fn quote_arg(arg: &str) -> String {
        if arg.is_empty() {
            return "\"\"".to_string();
        }
        if !arg.contains(' ') && !arg.contains('"') && !arg.contains('\t') {
            return arg.to_string();
        }
        let mut quoted = String::from('"');
        let mut backslashes = 0u32;
        for c in arg.chars() {
            if c == '\\' {
                backslashes += 1;
            } else if c == '"' {
                for _ in 0..backslashes {
                    quoted.push('\\');
                }
                quoted.push('\\');
                quoted.push('"');
                backslashes = 0;
            } else {
                backslashes = 0;
            }
            quoted.push(c);
        }
        for _ in 0..backslashes {
            quoted.push('\\');
        }
        quoted.push('"');
        quoted
    }
}
