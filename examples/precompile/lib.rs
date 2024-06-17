use fluentbase_sdk::{
    basic_entrypoint,
    codec::{Codec, Encoder},
    derive::{router, signature, Contract},
    AccountManager,
    Bytes,
    ContextReader,
    SharedAPI,
};

#[derive(Default, Codec)]
pub struct HelloWorldInput {}

#[derive(Default, Codec)]
pub struct HelloWorldOutput {
    message: Bytes,
}

pub trait PrecompileAPI {
    fn hello_world<SDK: SharedAPI>(&self, input: HelloWorldInput) -> HelloWorldOutput;
}

#[derive(Contract)]
pub struct PRECOMPILE<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

#[router(mode = "codec")]
impl<'a, CR: ContextReader, AM: AccountManager> PrecompileAPI for PRECOMPILE<'a, CR, AM> {
    #[signature("hello_world()")]
    fn hello_world<SDK: SharedAPI>(&self, _input: HelloWorldInput) -> HelloWorldOutput {
        HelloWorldOutput {
            message: "Hello, World".into(),
        }
    }
}

basic_entrypoint!(PRECOMPILE);
