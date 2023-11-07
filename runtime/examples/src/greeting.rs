use fluentbase_sdk::sys_write;

pub fn main() {
    let str = "Hello, World";
    sys_write(str.as_ptr() as u32, str.len() as u32);
}
