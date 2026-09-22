//! Where vtype keeps its files and which name its IPC endpoint has.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("com", "ishizakahiroshi", "vtype")
}

/// Directory of `config.json`. Falls back to the current directory when the OS gives no home.
pub fn config_dir() -> PathBuf {
    project_dirs()
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Directory of `vtype.log` (and, on macOS, the socket).
pub fn data_dir() -> PathBuf {
    project_dirs()
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

pub fn log_file() -> PathBuf {
    data_dir().join("vtype.log")
}

/// Operating system family, as a value so that path logic can be tested for every OS on any OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

impl Os {
    pub const fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Linux
        }
    }
}

/// Named pipe on Windows, Unix socket file elsewhere.
///
/// Windows: `\\.\pipe\vtype-<user>` (named pipes are machine-wide, so the user name keeps two
/// sessions apart). Linux: `$XDG_RUNTIME_DIR/vtype.sock`, else the data dir. macOS: the data dir.
pub fn ipc_endpoint_for(
    os: Os,
    user: &str,
    xdg_runtime_dir: Option<&Path>,
    data_dir: &Path,
) -> String {
    match os {
        Os::Windows => format!(r"\\.\pipe\vtype-{}", sanitize_user(user)),
        Os::Linux => match xdg_runtime_dir {
            Some(dir) if !dir.as_os_str().is_empty() => {
                dir.join("vtype.sock").to_string_lossy().into_owned()
            }
            _ => data_dir.join("vtype.sock").to_string_lossy().into_owned(),
        },
        Os::Mac => data_dir.join("vtype.sock").to_string_lossy().into_owned(),
    }
}

fn sanitize_user(user: &str) -> String {
    let cleaned: String = user
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "user".to_string()
    } else {
        cleaned
    }
}

pub fn ipc_endpoint() -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let xdg = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    ipc_endpoint_for(Os::current(), &user, xdg.as_deref(), &data_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_per_os() {
        let data = Path::new("/var/lib/vtype-test");
        assert_eq!(
            ipc_endpoint_for(Os::Windows, "Taro Y", None, data),
            r"\\.\pipe\vtype-Taro_Y"
        );
        assert_eq!(
            ipc_endpoint_for(Os::Windows, "", None, data),
            r"\\.\pipe\vtype-user"
        );
        assert_eq!(
            ipc_endpoint_for(Os::Linux, "a", Some(Path::new("/run/user/1000")), data),
            Path::new("/run/user/1000")
                .join("vtype.sock")
                .to_string_lossy()
        );
        assert_eq!(
            ipc_endpoint_for(Os::Linux, "a", None, data),
            data.join("vtype.sock").to_string_lossy()
        );
        assert_eq!(
            ipc_endpoint_for(Os::Mac, "a", Some(Path::new("/tmp")), data),
            data.join("vtype.sock").to_string_lossy()
        );
    }
}
