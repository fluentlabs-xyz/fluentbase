use fluentbase_sdk::evm::ExecutionContext;

pub fn deploy() {}

const HELLO_WORLD: [u8; 12] = [
    'H' as u8, 'e' as u8, 'l' as u8, 'l' as u8, 'o' as u8, ',' as u8, ' ' as u8, 'W' as u8,
    'o' as u8, 'r' as u8, 'l' as u8, 'd' as u8,
];

pub fn main() {
    let ctx = ExecutionContext::default();
    ctx.static_return_and_exit(&HELLO_WORLD, 0);
}
