#[allow(non_camel_case_types)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[cfg_attr(feature = "std", derive(strum_macros::EnumIter))]
pub enum SysFuncIdx {
    #[default]
    UNKNOWN = 0x0000,

    // crypto functions
    CRYPTO_KECCAK256 = 0x0101,
    // fluentbase_v1alpha::_sys_keccak256
    CRYPTO_POSEIDON = 0x0102,
    // fluentbase_v1alpha::_sys_poseidon
    CRYPTO_POSEIDON2 = 0x0103,
    // fluentbase_v1alpha::_sys_poseidon2
    CRYPTO_ECRECOVER = 0x0104, // fluentbase_v1alpha::_sys_ecrecover

    // SYS host functions (starts with 0x0000)
    SYS_HALT = 0x0001,
    // fluentbase_v1alpha::_sys_halt
    SYS_WRITE = 0x0005,
    // fluentbase_v1alpha::_sys_write
    SYS_INPUT_SIZE = 0x0004,
    // fluentbase_v1alpha::_sys_input_size
    SYS_READ = 0x0003,
    // fluentbase_v1alpha::_sys_read
    SYS_OUTPUT_SIZE = 0x0006,
    // fluentbase_v1alpha::_sys_output_size
    SYS_READ_OUTPUT = 0x0007,
    // fluentbase_v1alpha::_sys_read_output
    SYS_EXEC = 0x0008,
    // fluentbase_v1alpha::_sys_forward_output
    SYS_FORWARD_OUTPUT = 0x0009,
    // fluentbase_v1alpha::_sys_exec
    SYS_STATE = 0x0002, // fluentbase_v1alpha::_sys_state

    // jzkt functions
    JZKT_OPEN = 0x0701,
    JZKT_CHECKPOINT = 0x0702,
    JZKT_GET = 0x0703,
    JZKT_UPDATE = 0x0704,
    JZKT_UPDATE_PREIMAGE = 0x0705,
    JZKT_REMOVE = 0x0706,
    JZKT_COMPUTE_ROOT = 0x0707,
    JZKT_EMIT_LOG = 0x0708,
    JZKT_COMMIT = 0x0709,
    JZKT_ROLLBACK = 0x070A,
    JZKT_STORE = 0x070B,
    JZKT_LOAD = 0x070C,
    JZKT_PREIMAGE_SIZE = 0x070D,
    JZKT_PREIMAGE_COPY = 0x070E,
}

impl SysFuncIdx {
    pub fn fuel_cost(&self) -> u32 {
        match self {
            SysFuncIdx::SYS_HALT => 1,
            SysFuncIdx::SYS_STATE => 1,
            SysFuncIdx::SYS_READ => 1,
            SysFuncIdx::SYS_INPUT_SIZE => 1,
            SysFuncIdx::SYS_WRITE => 1,
            SysFuncIdx::CRYPTO_KECCAK256 => 1,
            SysFuncIdx::CRYPTO_POSEIDON => 1,
            SysFuncIdx::CRYPTO_POSEIDON2 => 1,
            SysFuncIdx::CRYPTO_ECRECOVER => 1,
            SysFuncIdx::JZKT_OPEN => 1,
            SysFuncIdx::JZKT_UPDATE => 1,
            SysFuncIdx::JZKT_GET => 1,
            SysFuncIdx::JZKT_COMPUTE_ROOT => 1,
            SysFuncIdx::JZKT_ROLLBACK => 1,
            SysFuncIdx::JZKT_COMMIT => 1,
            _ => 1, //unreachable!("not configured fuel for opcode: {:?}", self),
        }
    }
}

impl Into<u32> for SysFuncIdx {
    fn into(self) -> u32 {
        self as u32
    }
}
