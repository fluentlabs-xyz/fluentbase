use revm::{handler::EthFrame, interpreter::interpreter::EthInterpreter};

pub type RwasmFrame = EthFrame<EthInterpreter>;
