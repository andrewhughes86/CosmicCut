//! Where file reads and writes go. On the desktop that is the disk. In a browser there is no
//! disk: files the user uploads are held in memory under their names (`provide`), and every
//! write becomes a download the UI hands to the browser (`take_downloads`). Commands keep their
//! `path` parameters either way.

use std::io;

/// Reads a whole file of at most `cap` bytes; `Ok(None)` if it is larger.
pub fn read(path: &str, cap: u64) -> io::Result<Option<Vec<u8>>> {
    imp::read(path, cap)
}

/// Writes a whole file (in a browser, queues it as a download).
pub fn write(path: &str, bytes: &[u8]) -> io::Result<()> {
    imp::write(path, bytes)
}

/// Makes an uploaded file readable under `name`. Only the most recent few are kept.
pub fn provide(name: &str, bytes: Vec<u8>) {
    imp::provide(name, bytes);
}

/// The files written since the last call, as (file name, contents), oldest first.
pub fn take_downloads() -> Vec<(String, Vec<u8>)> {
    imp::take_downloads()
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::io::{self, Read as _};

    pub fn read(path: &str, cap: u64) -> io::Result<Option<Vec<u8>>> {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
        }
        if meta.len() > cap {
            return Ok(None);
        }
        // Read through a cap in case the file grows while it is read.
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        std::fs::File::open(path)?.take(cap.saturating_add(1)).read_to_end(&mut bytes)?;
        Ok((bytes.len() as u64 <= cap).then_some(bytes))
    }

    pub fn write(path: &str, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    pub fn provide(_name: &str, _bytes: Vec<u8>) {}

    pub fn take_downloads() -> Vec<(String, Vec<u8>)> {
        Vec::new()
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::cell::RefCell;
    use std::io;

    /// Uploaded files kept for reading; older ones are dropped.
    const KEPT: usize = 8;
    /// Downloads waiting for the UI; a stuck UI cannot make this grow without bound.
    const QUEUED: usize = 16;

    thread_local! {
        static UPLOADS: RefCell<Vec<(String, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
        static DOWNLOADS: RefCell<Vec<(String, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    }

    /// The file name: the browser has no folders.
    fn name(path: &str) -> &str {
        path.rsplit(['/', '\\']).next().unwrap_or(path)
    }

    pub fn read(path: &str, cap: u64) -> io::Result<Option<Vec<u8>>> {
        let name = name(path);
        UPLOADS.with_borrow(|files| {
            let (_, bytes) = files.iter().rev().find(|(n, _)| n == name).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "in the browser, open files with the Open or Import buttons")
            })?;
            Ok((bytes.len() as u64 <= cap).then(|| bytes.clone()))
        })
    }

    pub fn write(path: &str, bytes: &[u8]) -> io::Result<()> {
        let name = name(path).to_owned();
        DOWNLOADS.with_borrow_mut(|queue| {
            if queue.len() >= QUEUED {
                queue.remove(0);
            }
            queue.push((name, bytes.to_vec()));
        });
        Ok(())
    }

    pub fn provide(name: &str, bytes: Vec<u8>) {
        let name = self::name(name).to_owned();
        UPLOADS.with_borrow_mut(|files| {
            files.retain(|(n, _)| *n != name);
            if files.len() >= KEPT {
                files.remove(0);
            }
            files.push((name, bytes));
        });
    }

    pub fn take_downloads() -> Vec<(String, Vec<u8>)> {
        DOWNLOADS.with_borrow_mut(std::mem::take)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_are_capped() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("cosmiccut-io-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("f.txt");
        let path = path.to_string_lossy();
        super::write(&path, b"hello")?;
        assert_eq!(super::read(&path, 5)?, Some(b"hello".to_vec()));
        assert_eq!(super::read(&path, 4)?, None);
        assert!(super::read(&dir.to_string_lossy(), 99).is_err());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
