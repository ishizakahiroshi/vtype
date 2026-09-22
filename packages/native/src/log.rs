//! File logging for the long-running roles (daemon and host).
//!
//! Never log what the user said. Log the length of a transcript, not the transcript.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Appends to `path`; once it would pass `max` bytes the file becomes `<name>.1` (one generation).
pub struct RotatingFile {
    path: PathBuf,
    max: u64,
    file: Option<File>,
    size: u64,
}

impl RotatingFile {
    pub fn open(path: &Path, max: u64) -> io::Result<RotatingFile> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(RotatingFile { path: path.to_path_buf(), max, file: Some(file), size })
    }

    fn rotated_path(&self) -> PathBuf {
        let mut name = self.path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(".1");
        self.path.with_file_name(name)
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        let old = self.rotated_path();
        let _ = fs::remove_file(&old);
        fs::rename(&self.path, &old)?;
        self.file = Some(OpenOptions::new().create(true).append(true).open(&self.path)?);
        self.size = 0;
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.size > 0 && self.size + buf.len() as u64 > self.max {
            self.rotate()?;
        }
        let file = self.file.as_mut().ok_or_else(|| io::Error::other("log file closed"))?;
        let n = file.write(buf)?;
        self.size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

struct SharedWriter(&'static Mutex<RotatingFile>);

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).flush()
    }
}

/// Sends `tracing` output to the rotating log file. Falls back to stderr if the file cannot open.
pub fn init_file(role: &str) {
    let path = crate::paths::log_file();
    match RotatingFile::open(&path, MAX_LOG_BYTES) {
        Ok(file) => {
            let shared: &'static Mutex<RotatingFile> = Box::leak(Box::new(Mutex::new(file)));
            let _ = tracing_subscriber::fmt().with_ansi(false).with_writer(move || SharedWriter(shared)).try_init();
        }
        Err(_) => init_stderr(),
    }
    tracing::info!(role, version = env!("CARGO_PKG_VERSION"), "start");
}

pub fn init_stderr() {
    let _ = tracing_subscriber::fmt().with_writer(io::stderr).with_max_level(tracing::Level::WARN).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_one_generation() {
        let dir = std::env::temp_dir().join(format!("vtype-test-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("vtype.log");
        let mut f = RotatingFile::open(&path, 10).unwrap();
        f.write_all(b"12345678").unwrap();
        f.write_all(b"abcdef").unwrap(); // 8 + 6 > 10: rotate first
        f.write_all(b"XYZ").unwrap();
        f.flush().unwrap();
        drop(f);
        assert_eq!(fs::read_to_string(dir.join("vtype.log.1")).unwrap(), "12345678");
        assert_eq!(fs::read_to_string(&path).unwrap(), "abcdefXYZ");
        let _ = fs::remove_dir_all(&dir);
    }
}
