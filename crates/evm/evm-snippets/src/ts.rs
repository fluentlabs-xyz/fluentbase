use core::cell::OnceCell;
use fluentbase_sdk::evm::U256;
use hashbrown::HashMap;

static mut TS: OnceCell<HashMap<U256, U256>> = OnceCell::new();

pub fn ts_set(index: U256, value: U256) {
    unsafe {
        let mut v = TS.get_mut();
        if let Some(hm) = v {
            hm.insert(index, value);
        } else {
            let mut hm = HashMap::new();
            hm.insert(index, value);
            TS.set(hm).unwrap();
        }
    }
}

pub fn ts_get(index: U256) -> Option<U256> {
    unsafe {
        let mut hm = TS.get();
        if let Some(hm) = hm {
            let r = hm.get(&index);
            if let Some(_res) = r {
                return r.cloned();
            }
        }
        return None;
    }
}
