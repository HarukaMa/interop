# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Linux-Windows interop system. Executes Windows .exe files from Linux via binfmt_misc, with actual execution on the Windows side. Analogous to WSL's interop but works across any setup with shared files and network access (VMs, containers, separate machines).

## Build Commands

```bash
cargo build                              # build all crates
cargo build -p interop-protocol          # build only protocol
cargo build -p interop-server            # build only server
cargo build -p interop-client            # build only client
cargo build --release -p interop-server  # release build for server (Windows)
cargo build --release -p interop-client  # release build for client (Linux)
cargo test -p interop-protocol           # run protocol unit tests
cargo check                              # quick type-check all crates
```

The server is built on Windows (MSVC target), the client is cross-compiled or built on Linux. During development both may be built from MSYS2/Git Bash on Windows.

## Architecture

Three workspace crates communicating over TCP (port 62115) with length-prefixed binary frames (4-byte length + 1-byte type + payload):

**`protocol/`** — Shared library, no platform-specific code.
- `types.rs`: Wire types — `FrameType` enum (ExecRequest, StdinData, StdoutData, StderrData, ExitCode, StdinClose, PathQueryRequest, PathQueryResponse, Error, TerminalResize), `Frame`, `ExecRequest` (with `interactive`, `term_width`, `term_height` fields), `PathQueryResponse`
- `frame.rs`: `encode_frame`/`decode_frame` using `BytesMut`, plus convenience constructors (`stdout_data_frame`, `exit_code_frame`, etc.)
- `path.rs`: Bidirectional path translation. `/mnt/hgfs/X/...` ↔ `X:\...` (shared folders), `/path` ↔ `W:\path` (Linux root mapped to W: drive)
- `config.rs`: TOML config loading from `/etc/interop.toml` (Linux) or `%APPDATA%/interop/interop.toml` (Windows)

**`server/`** — Windows-side TCP server (multi-threaded tokio runtime).
- `connection.rs`: Per-connection handler with two exec paths: `handle_exec_interactive` (ConPTY) and `handle_exec_pipe` (pipes). Uses `tokio::select!` to race child exit vs client disconnect. On disconnect, child process is **not** killed (survives network loss).
- `conpty.rs`: Windows ConPTY pseudo-console spawning (`#[cfg(windows)]`). Uses `CreatePseudoConsole` + `CreateProcessW` with `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`. Pipes carry VT100 sequences. Blocking I/O via dedicated threads + `tokio::sync::mpsc` channels.
- `process.rs`: Non-interactive pipe-based process spawning. Uses `CREATE_NO_WINDOW` on MSVC targets.
- `service.rs`: Windows startup task integration behind `#[cfg(windows)]`. Install/uninstall via Task Scheduler (runs on logon).
- `main.rs`: CLI: `--console` (foreground dev mode), `--install`/`--uninstall` (startup task), default runs headless.

**`client/`** — Linux-side binfmt handler (single-threaded tokio runtime for fast startup).
- `exec.rs`: Connects to server, translates program path and args (Linux→Windows), streams I/O bidirectionally, returns exit code. Detects if stdin is a TTY to request interactive (ConPTY) mode. Handles SIGWINCH for terminal resize.
- `rawmode.rs`: Terminal raw mode management for interactive sessions. Enables raw mode when ConPTY is used so VT sequences pass through. RAII guard restores terminal on drop; SIGTERM handler for safety.
- `pathsync.rs`: Queries server for Windows PATH, translates to Linux paths, prints `export PATH=...` for shell eval.
- `main.rs`: Dispatch — no subcommand = exec mode (binfmt handler), `path-sync` subcommand.

## Key Design Details

- **Path translation heuristic**: Only args starting with `/`, `./`, `../` are translated. `--flag=/path` is split on `=`. The program path from binfmt is always translated.
- **binfmt_misc registration**: `.exe` extension match (not MZ magic — shared filesystems may show NTFS symlinks as empty files), `F` flag (fix binary — keeps fd open, requires `rm -f` before reinstall), no `P` flag (avoid extra argv[0]).
- **Disconnect behavior**: TCP drop does NOT kill the Windows process. Stdin pipe is closed, stdout/stderr are discarded, server logs when the orphaned process eventually exits.
- **StdinClose vs disconnect**: `stdin_task` returns `bool` — `true` = TCP dropped, `false` = graceful StdinClose. Only StdinClose case waits for child and sends exit code back.
- **Server under MSYS2**: `std::env::split_paths` is used for PATH splitting (handles both `;` and `:` separators). Path sync skips translating paths that are already Unix-style (no drive letter).
- **Two exec modes**: Interactive (stdin is TTY → ConPTY, raw mode, VT pass-through, single merged output stream) and non-interactive (piped stdin → separate stdout/stderr pipes, no terminal). Client auto-detects.
- **ConPTY I/O threading**: ConPTY pipes are synchronous Win32 handles. Blocking reads/writes run in dedicated `std::thread`s, bridged to async via `tokio::sync::mpsc` channels.
- **HANDLE as usize**: Windows `HANDLE` is `*mut c_void` on MinGW targets (not `Send`). All handles stored as `usize` in async code and cast back at call sites.
