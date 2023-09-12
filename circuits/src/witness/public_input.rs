use crate::{util::Field, witness::exit_code::UnrolledExitCode};

pub const N_PUBLIC_INPUT_BYTES: usize = 1;

#[derive(Clone, Default, Debug)]
pub struct UnrolledPublicInput<F: Field> {
    input: Vec<u8>,
    output: Vec<u8>,
    exit_code: UnrolledExitCode<F>,
}

impl<F: Field> UnrolledPublicInput<F> {
    pub fn new(input: &Vec<u8>, output: &Vec<u8>, exit_code: i32) -> Self {
        Self {
            input: input.clone(),
            output: output.clone(),
            exit_code: UnrolledExitCode::new(exit_code),
        }
    }

    pub fn input(&self) -> &Vec<u8> {
        &self.input
    }

    pub fn output(&self) -> &Vec<u8> {
        &self.output
    }

    pub fn exit_code(&self) -> &UnrolledExitCode<F> {
        &self.exit_code
    }
}
