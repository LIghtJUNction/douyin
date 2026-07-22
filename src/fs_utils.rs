use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    atomic_copy(&mut io::Cursor::new(contents), path).map(|_| ())
}

pub fn atomic_copy(reader: &mut impl Read, path: &Path) -> io::Result<u64> {
    let (temporary, mut output) = create_temporary_file(path)?;
    let result = (|| {
        let copied = io::copy(reader, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        drop(output);
        replace(&temporary, path)?;
        sync_parent(path)?;
        Ok(copied)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_temporary_file(path: &Path) -> io::Result<(PathBuf, File)> {
    let parent = parent_directory(path);
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("output");

    for _ in 0..100 {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".{name}.tmp-{}-{sequence}", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("无法为 {} 创建唯一临时文件", path.display()),
    ))
}

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn replace(temporary: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)
}

fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(parent_directory(path))?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{atomic_copy, atomic_write};
    use std::fs;
    use std::io::{self, Read};
    use std::path::PathBuf;

    fn test_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "douyin-fs-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn atomically_creates_and_replaces_files() {
        let directory = test_directory("replace");
        let path = directory.join("settings.json");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn removes_temporary_file_after_copy_failure() {
        struct FailingReader(bool);
        impl Read for FailingReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::other("expected failure"));
                }
                self.0 = true;
                buffer[..4].copy_from_slice(b"part");
                Ok(4)
            }
        }

        let directory = test_directory("cleanup");
        let path = directory.join("media.mp4");
        assert!(atomic_copy(&mut FailingReader(false), &path).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        fs::remove_dir_all(directory).unwrap();
    }
}
