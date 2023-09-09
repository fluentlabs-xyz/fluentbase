use crate::{
    util::Field,
    witness::{exit_code::UnrolledExitCode, raw_bytes::UnrolledRawBytes},
};

pub const N_PUBLIC_INPUT_BYTES: usize = 1;

#[derive(Clone, Default, Debug)]
pub struct UnrolledPublicInput<F: Field> {
    input: UnrolledRawBytes<F, N_PUBLIC_INPUT_BYTES>,
    output: UnrolledRawBytes<F, N_PUBLIC_INPUT_BYTES>,
    exit_code: UnrolledExitCode<F>,
}

impl<F: Field> UnrolledPublicInput<F> {
    pub fn new(input: &Vec<u8>, output: &Vec<u8>, exit_code: i32) -> Self {
        Self {
            input: UnrolledRawBytes::new(&input),
            output: UnrolledRawBytes::new(&output),
            exit_code: UnrolledExitCode::new(exit_code),
        }
    }

    pub fn input(&self) -> &UnrolledRawBytes<F, 1> {
        &self.input
    }

    pub fn output(&self) -> &UnrolledRawBytes<F, 1> {
        &self.output
    }

    pub fn exit_code(&self) -> &UnrolledExitCode<F> {
        &self.exit_code
    }
}
