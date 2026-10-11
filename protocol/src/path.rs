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
        format!(
            "{}\\{}",
            LINUX_ROOT_DRIVE,
            path.replacen('/', "", 1).replace('/', "\\")
        )
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
///
/// Single-component `/foo` arguments with 1-2 bytes after the slash are
/// treated as Windows switches. Root and multi-component paths qualify.
pub fn looks_like_path(s: &str) -> bool {
    if s.starts_with("./") || s.starts_with("../") {
        return true;
    }
    if let Some(rest) = s.strip_prefix('/') {
        return rest.is_empty() || rest.contains('/') || rest.len() >= 3;
    }
    false
}

/// Translate a single argument, handling `--flag=value` splitting.
pub fn translate_arg(arg: &str) -> String {
    // Try splitting on '=' for flags like --config=/path/to/file
    if let Some(eq_pos) = arg.find('=') {
        let (flag, value) = arg.split_at(eq_pos);
        let value = &value[1..]; // skip '='
        if value.starts_with('/') || looks_like_path(value) {
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

    #[test]
    fn test_looks_like_path_windows_switches() {
        assert!(!looks_like_path("/k"));
        assert!(!looks_like_path("/c"));
        assert!(!looks_like_path("/d"));
        assert!(!looks_like_path("/s"));
        assert!(!looks_like_path("/p"));
        assert!(!looks_like_path("/ab"));
    }

    #[test]
    fn test_looks_like_path_linux_dirs() {
        assert!(looks_like_path("/tmp"));
        assert!(looks_like_path("/etc"));
        assert!(looks_like_path("/bin"));
        assert!(looks_like_path("/usr"));
        assert!(looks_like_path("/opt"));
        assert!(looks_like_path("/home"));
        assert!(looks_like_path("/var"));
        assert!(looks_like_path("/dev"));
        assert!(looks_like_path("/mnt"));
    }

    #[test]
    fn test_looks_like_path_multicomponent() {
        assert!(looks_like_path("/home/user"));
        assert!(looks_like_path("/mnt/hgfs/C/file"));
        assert!(looks_like_path("/tmp/test.txt"));
        assert!(looks_like_path("./file"));
        assert!(looks_like_path("../dir"));
    }

    #[test]
    fn test_translate_arg_windows_switches_preserved() {
        assert_eq!(translate_arg("/k"), "/k");
        assert_eq!(translate_arg("/c"), "/c");
        assert_eq!(translate_arg("/d"), "/d");
    }

    #[test]
    fn test_translate_arg_linux_paths_translated() {
        assert_eq!(translate_arg("/tmp"), "W:\\tmp");
        assert_eq!(translate_arg("/etc"), "W:\\etc");
        assert_eq!(translate_arg("/home/user"), "W:\\home\\user");
        assert_eq!(translate_arg("/"), "W:\\");
    }

    #[test]
    fn test_translate_arg_short_explicit_paths() {
        assert_eq!(translate_arg("--output=/a"), "--output=W:\\a");
        assert_eq!(translate_arg("--output=/ab"), "--output=W:\\ab");
        assert_eq!(translate_arg("--output=/"), "--output=W:\\");
    }
}
