use rwasm::Caller;
use num::{BigUint, One, Zero};
use rwasm::core::Trap;
use sp1_curves::edwards::WORDS_FIELD_ELEMENT;
use sp1_primitives::consts::{bytes_to_words_le, words_to_bytes_le_vec, WORD_SIZE};

use crate::{RuntimeContext};

pub struct SyscallUint256Mul;

impl SyscallUint256Mul {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, arg1: u32, arg2: u32) -> Result<(), Trap> {
        let x_ptr = arg1;
        if x_ptr % 4 != 0 {
            panic!();
        }
        let y_ptr = arg2;
        if y_ptr % 4 != 0 {
            panic!();
        }

        // First read the words for the x value. We can read a slice_unsafe here because we write
        // the computed result to x later.
        let x = caller.read_memory(x_ptr, WORDS_FIELD_ELEMENT as u32)?;

        // Read the y value.
        let  y= caller.read_memory(y_ptr, WORDS_FIELD_ELEMENT as u32)?;

        // The modulus is stored after the y value. We increment the pointer by the number of words.
        let modulus_ptr = y_ptr + WORDS_FIELD_ELEMENT as u32 * WORD_SIZE as u32;
        let modulus= caller.read_memory(modulus_ptr, WORDS_FIELD_ELEMENT as u32)?;

        // Get the BigUint values for x, y, and the modulus.
        let uint256_x = BigUint::from_bytes_le(x);
        let uint256_y = BigUint::from_bytes_le(y);
        let uint256_modulus = BigUint::from_bytes_le(&modulus);

        // Perform the multiplication and take the result modulo the modulus.
        let result: BigUint = if uint256_modulus.is_zero() {
            let modulus = BigUint::one() << 256;
            (uint256_x * uint256_y) % modulus
        } else {
            (uint256_x * uint256_y) % uint256_modulus
        };

        let mut result_bytes = result.to_bytes_le();
        result_bytes.resize(32, 0u8); // Pad the result to 32 bytes.

        // Write the result to x and keep track of the memory records.
        caller.write_memory(x_ptr, &result_bytes)
    }
}
