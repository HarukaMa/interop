#!/bin/bash
set -euo pipefail

# VMware Interop Client Setup Script
# Run as root on the Linux VM guest.

INSTALL_DIR="/usr/local/bin"
CONFIG_FILE="/etc/interop.toml"
BINFMT_BASENAME="WSLInterop"
BINFMT_EXTENSIONS=("exe" "cmd" "bat" "ps1")
CLIENT_BIN="interop-client"

# Check for root
if [ "$(id -u)" -ne 0 ]; then
    echo "Error: This script must be run as root." >&2
    exit 1
fi

# Check if client binary exists in current directory or parent
if [ -f "./${CLIENT_BIN}" ]; then
    BIN_PATH="./${CLIENT_BIN}"
elif [ -f "../target/release/${CLIENT_BIN}" ]; then
    BIN_PATH="../target/release/${CLIENT_BIN}"
else
    echo "Error: Cannot find ${CLIENT_BIN} binary." >&2
    echo "Build it first: cargo build --release -p interop-client" >&2
    exit 1
fi

echo "=== VMware Interop Setup ==="

# 1. Unregister existing binfmt entries first (the F flag keeps the binary open)
echo "[1/5] Unregistering existing binfmt handlers..."
if [ -f "/proc/sys/fs/binfmt_misc/${BINFMT_BASENAME}" ]; then
    echo -1 > "/proc/sys/fs/binfmt_misc/${BINFMT_BASENAME}"
fi
for ext in "${BINFMT_EXTENSIONS[@]}"; do
    name="${BINFMT_BASENAME}_${ext}"
    if [ -f "/proc/sys/fs/binfmt_misc/${name}" ]; then
        echo -1 > "/proc/sys/fs/binfmt_misc/${name}"
    fi
done

# 2. Install binary (rm first to unlink the old inode — cp into a busy file fails)
echo "[2/5] Installing ${CLIENT_BIN} to ${INSTALL_DIR}..."
rm -f "${INSTALL_DIR}/${CLIENT_BIN}"
cp "${BIN_PATH}" "${INSTALL_DIR}/${CLIENT_BIN}"
chmod 755 "${INSTALL_DIR}/${CLIENT_BIN}"

# 3. Create config if it doesn't exist
if [ ! -f "${CONFIG_FILE}" ]; then
    echo "[3/5] Creating config at ${CONFIG_FILE}..."
    cat > "${CONFIG_FILE}" <<'EOF'
# VMware Interop Configuration
# host: IP address of the Windows host running interop-server
host = "172.16.0.1"
port = 62115
linux_root_drive = "W"
hgfs_prefix = "/mnt/hgfs/"
EOF
    echo "  -> Edit ${CONFIG_FILE} to set the correct Windows host IP."
else
    echo "[3/5] Config already exists at ${CONFIG_FILE}, skipping."
fi

# 4. Register binfmt_misc for Windows executable/script extensions
echo "[4/5] Registering binfmt_misc handlers for .exe/.cmd/.bat/.ps1 files..."

# Mount binfmt_misc if not already mounted
if ! mountpoint -q /proc/sys/fs/binfmt_misc 2>/dev/null; then
    mount -t binfmt_misc none /proc/sys/fs/binfmt_misc || true
fi

# Register: match extensions (not MZ magic) because VMware HGFS
# may present hardlinked files as empty (0 bytes), so magic-based
# matching fails for those executables.
# Flags: F = fix binary (use registered path even in other mount namespaces)
for ext in "${BINFMT_EXTENSIONS[@]}"; do
    name="${BINFMT_BASENAME}_${ext}"
    echo ":${name}:E::${ext}::${INSTALL_DIR}/${CLIENT_BIN}:F" > /proc/sys/fs/binfmt_misc/register
    echo "  -> Registered .${ext} handler via binfmt_misc (${name})"
done

# 4. Shell integration instructions
echo "[5/5] Setup complete!"
echo ""
echo "=== Next Steps ==="
echo ""
echo "1. Ensure interop-server.exe is running on the Windows host:"
echo "   interop-server.exe --console"
echo ""
echo "2. Edit ${CONFIG_FILE} and set 'host' to your Windows host IP."
echo "   Find it with: ip route | grep default"
echo ""
echo "3. To sync Windows PATH, add to your ~/.bashrc:"
echo '   eval "$(interop-client path-sync)"'
echo ""
echo "4. Test: notepad.exe or cmd.exe /c dir"
