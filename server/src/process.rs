use std::process::Stdio;
use tokio::process::{Child, Command};

/// Spawn a Windows process with piped I/O.
#[allow(unused_mut)]
pub fn spawn_process(
    program: &str,
    args: &[String],
    cwd: &str,
    env: &[(String, String)],
) -> std::io::Result<Child> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if !cwd.is_empty() {
        cmd.current_dir(cwd);
    }

    for (key, value) in env {
        cmd.env(key, value);
    }

    // On Windows, use CREATE_NO_WINDOW to prevent console popups
    #[cfg(windows)]
    {
        #[allow(unused_imports)]
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        #[cfg(not(target_env = "gnu"))]
        cmd.creation_flags(CREATE_NO_WINDOW);
        #[cfg(target_env = "gnu")]
        let _ = CREATE_NO_WINDOW; // CommandExt not available in MinGW
    }

    cmd.spawn()
}
