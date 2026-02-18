# linux-windows-interop

Run Windows executables seamlessly from Linux, with actual execution on a Windows machine. Analogous to WSL's Windows interop, but works across any setup where the two systems can share files and reach each other over the network — VMs (VMware, VirtualBox, QEMU/KVM, Hyper-V), containers, or even two separate physical machines.

```
Linux                             Windows
$ notepad.exe readme.txt    -->   notepad.exe C:\...\readme.txt
$ cargo.exe build           -->   cargo.exe build (in translated cwd)
$ uv.exe pip install foo    -->   uv.exe pip install foo
```

> **This project was built entirely by AI** (Claude / Claude Code). The code, architecture, and documentation were all AI-generated. While functional, it has not been through a formal security audit.

> **Security warning**: The server listens on a TCP port and executes arbitrary commands **with no authentication or encryption**. Anyone who can reach the port gets full shell access as the server's user. Only run this on trusted networks. See [Security](#security) for details.

(human note: all with claude opus 4.6 in claude code. burn through only 3 full sessions of claude pro. take this as a demo of what the current state-of-the-art code agent could do. yes, i do use this thing on my computer. i didn't check too many implementation details (i know nothing about conpty actually). i did check the content of readme, most if not all of it should be accurate.)

## How it works

1. Linux kernel's `binfmt_misc` intercepts `.exe` file execution
2. The **interop-client** on Linux connects to the **interop-server** on Windows over TCP (port 62115)
3. File paths and arguments are automatically translated between Linux and Windows conventions
4. stdin/stdout/stderr are streamed bidirectionally
5. Interactive programs get a full Windows pseudo-terminal (ConPTY) with VT100 pass-through

### Path translation

| Linux path | Windows path |
|---|---|
| `/mnt/hgfs/C/Users/me/file.txt` | `C:\Users\me\file.txt` |
| `/home/user/project` | `W:\home\user\project` |
| `./relative/path` | translated relative to CWD |

The shared filesystem mount prefix and drive letter mapping are configurable. By default, `/mnt/hgfs/X/...` maps to `X:\...` (VMware-style). Other Linux paths map to a configurable drive letter (`W:` by default, expecting the Linux root exported to Windows as a network drive or similar). Both the prefix and drive letter can be changed in `interop.toml` to match any shared filesystem layout.

### Two execution modes

- **Interactive** (stdin is a TTY): Uses Windows ConPTY. Full terminal support with colors, cursor movement, resize handling. Raw mode on the Linux side passes VT sequences through transparently.
- **Non-interactive** (piped stdin): Separate stdout/stderr streams, no terminal emulation.

## Architecture

Three Rust workspace crates:

```
protocol/     Shared types, frame encoding, path translation, config
server/       Windows-side TCP server (tokio, ConPTY, Windows Service support)
client/       Linux-side binfmt handler (single-threaded tokio for fast startup)
```

Communication uses length-prefixed binary frames (4-byte length + 1-byte type tag + payload) over TCP.

## Setup

### Prerequisites

- Shared filesystem between Linux and Windows (VMware HGFS, NFS, SMB/CIFS, SSHFS, etc.)
- Network connectivity between the two machines (VM NAT/host-only, LAN, etc.)
- Rust toolchain on both platforms (or cross-compilation)

### Build

```bash
# On Windows — build the server
cargo build --release -p interop-server

# On Linux — build the client
cargo build --release -p interop-client
```

### Install — Windows host

```bash
# Development (foreground with logging):
interop-server.exe --console

# Install as startup task (runs on logon via Task Scheduler):
interop-server.exe --install

# Remove startup task:
interop-server.exe --uninstall
```

Config: `%APPDATA%\interop\interop.toml`

### Install — Linux guest

```bash
sudo bash scripts/setup.sh
```

This installs the client binary, registers the `binfmt_misc` handler (by `.exe` extension), and creates `/etc/interop.toml`. Edit the config to set your Windows host IP:

```toml
host = "172.16.0.1"    # IP address of the Windows machine running the server
port = 62115
linux_root_drive = "W"
hgfs_prefix = "/mnt/hgfs/"
```

### Shell integration

Add to `~/.bashrc` or `~/.zshrc` to make Windows PATH entries available on Linux:

```bash
eval "$(interop-client path-sync)"
```

Tip: if you reload your shell frequently (`exec zsh`), save the original PATH first to avoid duplication:

```bash
if [[ -z "$_ORIG_PATH" ]]; then
    export _ORIG_PATH="$PATH"
fi
PATH="$_ORIG_PATH"

eval "$(interop-client path-sync)"
```

## Usage

Once set up, just run `.exe` files as if they were native:

```bash
notepad.exe file.txt
cmd.exe /c dir
cargo.exe build
python.exe script.py
```

Path arguments starting with `/`, `./`, or `../` are automatically translated. Flag arguments like `--output=/some/path` are split on `=` and the path portion is translated.

## Known limitations

- **NTFS symlinks invisible through shared filesystems**: Some shared filesystems (notably VMware HGFS) cannot resolve NTFS symlinks and present them as empty (0-byte) files. This affects tools like `rustc.exe` (a symlink to `rustup.exe`). The extension-based binfmt registration works around this — the file doesn't need to be readable on Linux since it runs on Windows.
- **ConPTY process lifetime**: Interactive processes are tied to the server's ConPTY handle. If the server exits, interactive processes lose their console and typically exit. Non-interactive (piped) processes survive.
- **No reconnection**: If the TCP connection drops, the Windows process continues running but its output is discarded. There is no mechanism to reattach.
- **Single output stream in interactive mode**: ConPTY merges stdout and stderr into one stream, as real terminals do.

## Security

**This project opens a TCP port (62115) that allows remote command execution on the Windows host.** Understand the implications before using it.

- **No authentication**: Any process that can reach the server's TCP port can execute arbitrary commands as the server's user. The server is designed for a trusted local VM-to-host link, not for exposure to untrusted networks.
- **No encryption**: All traffic (commands, arguments, stdin/stdout data) is sent in plaintext over TCP.
- **Full user privileges**: Commands run with the full privileges of the Windows user running the server.
- **AI-generated code**: This entire project — including the protocol, server, client, and setup scripts — was generated by AI (Claude). It has not been audited for security vulnerabilities. There may be bugs in frame parsing, path translation, handle management, or other areas that could be exploitable.
- **Recommended mitigations**:
  - Only bind to the VM host-only or NAT interface, not to public networks
  - Use firewall rules to restrict access to the server port
  - Do not run the server as Administrator unless necessary
  - Treat this as a development/convenience tool, not a production system

## License

Unlicensed / personal project.
