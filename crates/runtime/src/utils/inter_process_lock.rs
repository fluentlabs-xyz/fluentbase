use fluentbase_types::B256;
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
};

pub fn inter_process_lock(id: String) -> File {
    // Put the lock file in target dir to avoid VCS noise
    let mut p = PathBuf::from(env!("CARGO_TARGET_DIR"));
    p.push(format!("runtime-{}.lock", id));
    let f = File::create(p).expect("create lock file");
    f.lock_exclusive()
        .expect("acquire global interprocess lock");
    f
}
