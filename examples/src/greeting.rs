use fluentbase_sdk::evm::ExecutionContext;

pub fn main() {
    let mut ctx = ExecutionContext::default();
    ctx.return_and_exit("Hello, World".as_bytes(), 0);
}
