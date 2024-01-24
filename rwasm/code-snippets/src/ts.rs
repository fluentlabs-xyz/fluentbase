use core::{cell::OnceCell, ptr::null_mut};
use fluentbase_sdk::evm::U256;
use hashbrown::HashMap;

static mut TS: *mut HashMap<U256, U256> = null_mut();
static mut TS1: OnceCell<HashMap<U256, U256>> = OnceCell::new();

pub fn ts_set(index: U256, value: U256) {
    unsafe {
        let mut v = TS1.get_mut();
        if let Some(hm) = v {
            hm.insert(index, value);
        } else {
            let mut hm = HashMap::new();
            hm.insert(index, value);
            TS1.set(hm).unwrap();
        }
    }
}

pub fn ts_get(index: U256) -> Option<U256> {
    unsafe {
        let mut hm = TS1.get();
        if let Some(hm) = hm {
            let r = hm.get(&index);
            if let Some(res) = r {
                return r.cloned();
            }
        }
        return None;
    }
}
