use fluentbase_types::B256;
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
};

pub const FILE_NAME_PREFIX1: &str = "runtime-wasm-module";

fn lock_id_str_file_path(file_name_prefix: &str, s: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("{}-{}.lock", file_name_prefix, s));
    path
}

fn lock_id_u64_file_path(file_name_prefix: &str, id: u64) -> PathBuf {
    lock_id_str_file_path(file_name_prefix, &id.to_string())
}

fn lock_id_b256_file_path(file_name_prefix: &str, id: &B256) -> PathBuf {
    lock_id_str_file_path(file_name_prefix, &id.to_string())
}

pub struct InterProcessLock {
    file: File,
}

impl InterProcessLock {
    pub fn acquire(path: PathBuf) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)?;

        // blocks until lock is acquired
        file.lock_exclusive()?;

        Ok(Self { file })
    }
    pub fn acquire_on_u64(file_name_prefix: &str, id: u64) -> std::io::Result<Self> {
        let path = lock_id_u64_file_path(file_name_prefix, id);
        Self::acquire(path)
    }
    pub fn acquire_on_b256(file_name_prefix: &str, id: &B256) -> std::io::Result<Self> {
        let path = lock_id_b256_file_path(file_name_prefix, id);
        Self::acquire(path)
    }
}

impl Drop for InterProcessLock {
    fn drop(&mut self) {
        #[allow(unstable_name_collisions)]
        let _ = self.file.unlock();
    }
}
