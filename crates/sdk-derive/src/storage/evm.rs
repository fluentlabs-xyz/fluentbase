trait Storage {
    fn sload(&self, key: U256) -> U256 {}
    fn sstore(&self, key: U256, value: U256) {}
}

trait LE32Bytes {
    fn is_le_32_bytes(ty: V) -> V {}
}

trait Value {}

trait StorageValue {
    // load full value (probably more than 1 word (32 bytes)
    fn load(key: LE32Bytes) -> Value {}
    fn store(key: LE32Bytes, value: Value) {}
}

struct Value;
