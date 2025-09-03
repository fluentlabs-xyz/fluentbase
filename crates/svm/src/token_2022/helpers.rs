use solana_program_error::ProgramError;

pub fn next_item<'a, 'b, T, I: Iterator<Item = &'a T>>(
    iter: &mut I,
) -> Result<I::Item, ProgramError> {
    iter.next().ok_or(ProgramError::NotEnoughAccountKeys)
}
