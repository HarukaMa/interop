mod exec;
mod pathsync;
mod rawmode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "interop-client", about = "VMware Interop Client")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Program to execute (used as binfmt handler)
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Synchronize Windows PATH to Linux
    PathSync,
}

fn main() {
    // Install signal handler to restore terminal on unexpected termination
    #[cfg(unix)]
    {
        unsafe {
            libc::signal(libc::SIGTERM, restore_terminal_signal as libc::sighandler_t);
            // Don't install SIGINT handler — in raw mode SIGINT won't be generated
            // by the terminal anyway (ISIG is off). If somehow received, default
            // behavior (terminate) is fine since the drop guard runs.
        }
    }

    let cli = Cli::parse();

    // Single-threaded runtime for fast binfmt startup
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    match cli.command {
        Some(Commands::PathSync) => {
            let code = rt.block_on(pathsync::run_path_sync());
            std::process::exit(code);
        }
        None => {
            if cli.args.is_empty() {
                eprintln!("Usage: interop-client <program> [args...]");
                eprintln!("       interop-client path-sync");
                std::process::exit(1);
            }

            let program = cli.args[0].clone();
            let args = cli.args[1..].to_vec();

            let code = rt.block_on(exec::run_exec(program, args));
            std::process::exit(code);
        }
    }
}

#[cfg(unix)]
extern "C" fn restore_terminal_signal(_sig: libc::c_int) {
    rawmode::restore_from_signal();
    // Re-raise with default handler to get proper exit status
    unsafe {
        libc::signal(_sig, libc::SIG_DFL);
        libc::raise(_sig);
    }
}
