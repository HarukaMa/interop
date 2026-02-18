#![cfg_attr(windows, windows_subsystem = "windows")]

mod connection;
mod conpty;
mod process;
mod service;

use clap::Parser;
use interop_protocol::Config;
use tokio::net::TcpListener;
use tracing::info;

#[derive(Parser)]
#[command(name = "interop-server", about = "VMware Interop Server")]
struct Cli {
    /// Run in console mode (foreground, for development)
    #[arg(long)]
    console: bool,

    /// Install as a startup task (Task Scheduler, runs on logon)
    #[arg(long)]
    install: bool,

    /// Remove the startup task
    #[arg(long)]
    uninstall: bool,
}

pub async fn run_listener() {
    let config = Config::load();
    let addr = format!("0.0.0.0:{}", config.port);

    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind {addr}: {e}");
            return;
        }
    };
    info!("Listening on {addr}");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                tokio::spawn(connection::handle_connection(stream, peer));
            }
            Err(e) => {
                tracing::error!("Accept error: {e}");
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();

    if cli.install {
        #[cfg(windows)]
        {
            if let Err(e) = service::win::install_startup() {
                eprintln!("Install failed: {e}");
                std::process::exit(1);
            }
        }
        #[cfg(not(windows))]
        {
            eprintln!("Startup install is only supported on Windows");
            std::process::exit(1);
        }
        return;
    }

    if cli.uninstall {
        #[cfg(windows)]
        {
            if let Err(e) = service::win::uninstall_startup() {
                eprintln!("Uninstall failed: {e}");
                std::process::exit(1);
            }
        }
        #[cfg(not(windows))]
        {
            eprintln!("Startup uninstall is only supported on Windows");
            std::process::exit(1);
        }
        return;
    }

    if cli.console {
        // Attach to parent console (e.g. cmd/powershell that launched us)
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Console::{AttachConsole, AllocConsole, ATTACH_PARENT_PROCESS};
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                AllocConsole();
            }
        }

        tracing_subscriber::fmt::init();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(run_listener());
        return;
    }

    // Default: headless mode (Task Scheduler / background)
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(run_listener());
}
