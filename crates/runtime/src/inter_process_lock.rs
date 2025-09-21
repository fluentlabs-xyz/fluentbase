use fluentbase_types::B256;
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
};

pub const FILE_NAME_PREFIX1: &str = "runtime-wasm-module";

pub struct InterProcessLock {
    file: File,
}

impl InterProcessLock {
    fn acquire_inner(path: PathBuf) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)?;

        // blocks until lock is acquired
        file.lock_exclusive()?;

        Ok(Self { file })
    }

    pub fn acquire(file_name_prefix: &str, id: String) -> std::io::Result<Self> {
        let mut path = std::env::temp_dir();
        path.push(format!("{}-{}.lock", file_name_prefix, id));
        Self::acquire_inner(path)
    }
}

impl Drop for InterProcessLock {
    fn drop(&mut self) {
        #[allow(unstable_name_collisions)]
        let _ = self.file.unlock();
    }
}
