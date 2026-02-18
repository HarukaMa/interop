/// Startup registration via Task Scheduler.
///
/// Uses schtasks to register the server as a logon task that runs in the
/// user's interactive session (required for ConPTY and GUI programs).
#[cfg(windows)]
pub mod win {
    use std::process::Command;

    const TASK_NAME: &str = "InteropServer";

    pub fn install_startup() -> Result<(), Box<dyn std::error::Error>> {
        let exe_path = std::env::current_exe()?;
        let exe_str = exe_path.to_str().ok_or("Invalid exe path")?;

        // Remove existing task if present (ignore errors)
        let _ = Command::new("schtasks")
            .args(["/delete", "/tn", TASK_NAME, "/f"])
            .output();

        let status = Command::new("schtasks")
            .args([
                "/create",
                "/tn", TASK_NAME,
                "/tr", &format!("\"{exe_str}\""),
                "/sc", "onlogon",
                "/rl", "highest",
            ])
            .status()?;

        if !status.success() {
            return Err("schtasks /create failed".into());
        }

        println!("Startup task '{TASK_NAME}' installed.");
        println!("Binary: {exe_str}");
        println!("The server will start automatically on logon.");
        println!("To start now: schtasks /run /tn {TASK_NAME}");
        Ok(())
    }

    pub fn uninstall_startup() -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("schtasks")
            .args(["/delete", "/tn", TASK_NAME, "/f"])
            .status()?;

        if !status.success() {
            return Err("schtasks /delete failed".into());
        }

        println!("Startup task '{TASK_NAME}' removed.");
        Ok(())
    }
}
