use crate::byteorder::{BigEndian, ByteOrder};

#[inline(always)]
pub fn write_evm_exit_message<R, F: FnMut(&[u8]) -> R>(exit_code: u32, mut write_func: F) -> R {
    // we use Solidity 0.8 compatible error format where the first 4 bytes is signature,
    // and the last 4 bytes is error code
    let mut output: [u8; 4 + 32] = [
        0x4e, 0x48, 0x7b, 0x71, // 4 byte signature `Panic(uint256)` - 0..4
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, // 32 bytes error code (only last 4 bytes used) - 4..36
    ];
    BigEndian::write_u32(&mut output[32..], exit_code);
    // write buffer into output
    write_func(&output)
}

#[inline(always)]
pub fn write_evm_panic_message<F: FnMut(&[u8])>(panic_message: &str, mut write_func: F) {
    // we use Solidity 0.8 compatible error format where the first 4 bytes is signature,
    // and the last
    let mut output: [u8; 4 + 32 + 32] = [
        0x08, 0xc3, 0x79, 0xa0, // 4 byte signature `Error(string)` - 0..4
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x20, // 32 byte array offset - 4..36
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, // length of the string - (36+32-4...)..(36+32)
    ];
    BigEndian::write_u32(
        &mut output[(36 + 32 - 4)..(36 + 32)],
        panic_message.len() as u32,
    );
    // write the header of the ABI message into output
    write_func(&output);
    // write each message chunk into output
    for chunk in panic_message.as_bytes().chunks(32) {
        // write chunk into output
        write_func(chunk);
        // if we need to pad remaining bytes then fill it with zeroes
        let padding_len = 32 - chunk.len();
        if padding_len == 0 {
            continue;
        }
        const ZEROS: [u8; 32] = [0u8; 32];
        write_func(&ZEROS[..padding_len]);
    }
}

#[cfg(test)]
mod tests {
    use crate::evm::{write_evm_exit_message, write_evm_panic_message};
    use alloy_primitives::hex;

    #[test]
    fn test_evm_exit() {
        let output = write_evm_exit_message(123, |slice| slice.to_vec());
        assert_eq!(
            output,
            hex!("4e487b71000000000000000000000000000000000000000000000000000000000000007b")
        );
    }

    #[test]
    fn test_evm_panic() {
        let mut output = vec![];
        write_evm_panic_message("Hello, World", |slice| {
            output.extend_from_slice(slice);
        });
        assert_eq!(output, hex!("08c379a00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000c48656c6c6f2c20576f726c640000000000000000000000000000000000000000"));
        output.clear();
        write_evm_panic_message("Hello, World, Hello, World, Hello, World", |slice| {
            output.extend_from_slice(slice);
        });
        assert_eq!(output, hex!("08c379a00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000002848656c6c6f2c20576f726c642c2048656c6c6f2c20576f726c642c2048656c6c6f2c20576f726c64000000000000000000000000000000000000000000000000"));
    }
}
