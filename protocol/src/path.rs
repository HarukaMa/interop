/// Shared folder prefix in Linux (VMware HGFS mounts).
const HGFS_PREFIX: &str = "/mnt/hgfs/";

/// Drive letter used to map the Linux root on Windows.
const LINUX_ROOT_DRIVE: &str = "W:";

/// Convert a Linux path to a Windows path.
///
/// - `/mnt/hgfs/C/Users/foo` → `C:\Users\foo`
/// - `/home/user/file` → `W:\home\user\file`
pub fn linux_to_windows(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(HGFS_PREFIX) {
        // rest = "C/Users/foo" → drive = "C", remainder = "Users/foo"
        if let Some(slash_pos) = rest.find('/') {
            let drive = &rest[..slash_pos];
            let remainder = &rest[slash_pos + 1..];
            format!("{}:\\{}", drive, remainder.replace('/', "\\"))
        } else {
            // Just the drive root, e.g. /mnt/hgfs/C
            format!("{}:\\", rest)
        }
    } else {
        format!("{}\\{}", LINUX_ROOT_DRIVE, path.replacen('/', "", 1).replace('/', "\\"))
    }
}

/// Convert a Windows path to a Linux path.
///
/// - `C:\Users\foo` → `/mnt/hgfs/C/Users/foo`
/// - `W:\home\user` → `/home/user`
pub fn windows_to_linux(path: &str) -> String {
    // Normalize to forward slashes first
    let normalized = path.replace('\\', "/");

    // Check for drive letter pattern like "X:/" or "X:"
    if normalized.len() >= 2 && normalized.as_bytes()[1] == b':' {
        let drive = &normalized[..1];
        let rest = if normalized.len() > 2 {
            &normalized[2..] // skip ":"
        } else {
            "/"
        };
        let rest = rest.strip_prefix('/').unwrap_or(rest);

        if drive.eq_ignore_ascii_case("W") {
            // Linux root drive
            if rest.is_empty() {
                "/".to_string()
            } else {
                format!("/{rest}")
            }
        } else {
            format!("/mnt/hgfs/{drive}/{rest}")
        }
    } else {
        // Not a drive path, return as-is with forward slashes
        normalized
    }
}

/// Heuristic: does this string look like a path that should be translated?
pub fn looks_like_path(s: &str) -> bool {
    s.starts_with('/') || s.starts_with("./") || s.starts_with("../")
}

/// Translate a single argument, handling `--flag=value` splitting.
pub fn translate_arg(arg: &str) -> String {
    // Try splitting on '=' for flags like --config=/path/to/file
    if let Some(eq_pos) = arg.find('=') {
        let (flag, value) = arg.split_at(eq_pos);
        let value = &value[1..]; // skip '='
        if looks_like_path(value) {
            return format!("{}={}", flag, linux_to_windows(value));
        }
    }

    if looks_like_path(arg) {
        linux_to_windows(arg)
    } else {
        arg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linux_to_windows_hgfs() {
        assert_eq!(linux_to_windows("/mnt/hgfs/C/Users/foo"), "C:\\Users\\foo");
        assert_eq!(linux_to_windows("/mnt/hgfs/D/"), "D:\\");
        assert_eq!(linux_to_windows("/mnt/hgfs/C"), "C:\\");
    }

    #[test]
    fn test_linux_to_windows_root() {
        assert_eq!(linux_to_windows("/tmp/test.txt"), "W:\\tmp\\test.txt");
        assert_eq!(linux_to_windows("/home/user"), "W:\\home\\user");
    }

    #[test]
    fn test_windows_to_linux_drive() {
        assert_eq!(windows_to_linux("C:\\Users\\foo"), "/mnt/hgfs/C/Users/foo");
        assert_eq!(windows_to_linux("D:\\"), "/mnt/hgfs/D/");
    }

    #[test]
    fn test_windows_to_linux_root_drive() {
        assert_eq!(windows_to_linux("W:\\tmp\\test.txt"), "/tmp/test.txt");
    }

    #[test]
    fn test_translate_arg_flag() {
        assert_eq!(
            translate_arg("--config=/etc/app.conf"),
            "--config=W:\\etc\\app.conf"
        );
    }

    #[test]
    fn test_translate_arg_plain() {
        assert_eq!(translate_arg("hello"), "hello");
        assert_eq!(translate_arg("-v"), "-v");
    }
}
