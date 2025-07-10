use crate::solana_program::{
    instruction::SVMInstruction,
    message::{AccountKeys, SanitizedMessage},
    svm_message::SVMMessage,
};
use solana_pubkey::Pubkey;

// Implement for the "reference" `SanitizedMessage` type.
impl SVMMessage for SanitizedMessage {
    fn num_instructions(&self) -> usize {
        SanitizedMessage::instructions(self).len()
    }

    fn instructions_iter(&self) -> impl Iterator<Item = SVMInstruction> {
        SanitizedMessage::instructions(self)
            .iter()
            .map(SVMInstruction::from)
    }

    fn program_instructions_iter(&self) -> impl Iterator<Item = (&Pubkey, SVMInstruction)> + Clone {
        SanitizedMessage::program_instructions_iter(self)
            .map(|(pubkey, ix)| (pubkey, SVMInstruction::from(ix)))
    }

    fn account_keys(&self) -> AccountKeys {
        SanitizedMessage::account_keys(self)
    }

    fn fee_payer(&self) -> &Pubkey {
        SanitizedMessage::fee_payer(self)
    }

    fn is_writable(&self, index: usize) -> bool {
        SanitizedMessage::is_writable(self, index)
    }

    fn is_signer(&self, index: usize) -> bool {
        SanitizedMessage::is_signer(self, index)
    }
}
