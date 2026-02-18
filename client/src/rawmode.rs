/// Terminal raw mode for interactive ConPTY sessions.
///
/// In raw mode, all input (keystrokes, escape sequences, control characters)
/// is forwarded to the remote process via ConPTY. The ConPTY translates them
/// to Windows Console events. Output from ConPTY is VT100 sequences that the
/// Linux terminal renders directly.

#[cfg(unix)]
mod unix {
    use std::os::unix::io::AsRawFd;
    use std::sync::OnceLock;

    static ORIGINAL_TERMIOS: OnceLock<libc::termios> = OnceLock::new();

    pub struct RawModeGuard {
        fd: i32,
    }

    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            restore_terminal(self.fd);
        }
    }

    /// Enable raw mode on stdin if it's a TTY.
    pub fn enable_raw_mode() -> Option<RawModeGuard> {
        let fd = std::io::stdin().as_raw_fd();

        if unsafe { libc::isatty(fd) } != 1 {
            return None;
        }

        let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return None;
        }
        let _ = ORIGINAL_TERMIOS.set(original);

        let mut raw = original;
        raw.c_iflag &= !(libc::IGNBRK
            | libc::BRKINT
            | libc::PARMRK
            | libc::ISTRIP
            | libc::INLCR
            | libc::IGNCR
            | libc::ICRNL
            | libc::IXON);
        raw.c_oflag &= !libc::OPOST;
        raw.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN);
        raw.c_cflag &= !(libc::CSIZE | libc::PARENB);
        raw.c_cflag |= libc::CS8;
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;

        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &raw) } != 0 {
            return None;
        }

        Some(RawModeGuard { fd })
    }

    /// Check if stdin is a TTY.
    pub fn is_tty() -> bool {
        unsafe { libc::isatty(std::io::stdin().as_raw_fd()) == 1 }
    }

    /// Get the current terminal size (cols, rows).
    pub fn terminal_size() -> (u16, u16) {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
                (ws.ws_col, ws.ws_row)
            } else {
                (80, 24)
            }
        }
    }

    fn restore_terminal(fd: i32) {
        if let Some(original) = ORIGINAL_TERMIOS.get() {
            unsafe {
                libc::tcsetattr(fd, libc::TCSAFLUSH, original);
            }
        }
    }

    /// Restore terminal from signal handler context (async-signal-safe).
    pub fn restore_from_signal() {
        if let Some(original) = ORIGINAL_TERMIOS.get() {
            unsafe {
                libc::tcsetattr(0, libc::TCSAFLUSH, original);
            }
        }
    }
}

#[cfg(unix)]
pub use unix::*;

#[cfg(not(unix))]
pub struct RawModeGuard;

#[cfg(not(unix))]
#[allow(dead_code)]
pub fn enable_raw_mode() -> Option<RawModeGuard> {
    None
}

#[cfg(not(unix))]
#[allow(dead_code)]
pub fn is_tty() -> bool {
    false
}

#[cfg(not(unix))]
#[allow(dead_code)]
pub fn terminal_size() -> (u16, u16) {
    (80, 24)
}

#[cfg(not(unix))]
#[allow(dead_code)]
pub fn restore_from_signal() {}
