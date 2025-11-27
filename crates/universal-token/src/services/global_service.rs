use crate::services::global::GlobalService;
use spin::{Mutex, MutexGuard};

pub static GLOBAL_SERVICE: spin::Once<Mutex<GlobalService>> = spin::Once::new();

pub fn global_service<'a>() -> MutexGuard<'a, GlobalService> {
    let v = GLOBAL_SERVICE.call_once(|| {
        let service = GlobalService::new();
        Mutex::new(service)
    });
    v.lock()
}
