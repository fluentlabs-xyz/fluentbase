pub const FD_STDOUT: u32 = 0;
pub const FD_STDERR: u32 = 1;
// We skip [2-14] here, because we want to satisfy the latest SP1 hooks ids
pub const FD_ECRECOVER_HOOK: u32 = 15;
pub const FD_ED_DECOMPRESS: u32 = 16;
pub const FD_RSA_MUL_MOD: u32 = 17;
pub const FD_BLS12_381_SQRT: u32 = 18;
pub const FD_BLS12_381_INVERSE: u32 = 19;
pub const FD_FP_SQRT: u32 = 20;
pub const FD_FP_INV: u32 = 21;
