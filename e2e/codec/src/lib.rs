//! Runtime support for the corpus cases that `build.rs` generates.
//!
//! The corpus is `@ethersproject/testcases`, whose expected bytes were produced by compiling
//! generated Solidity with solc and executing it on a node - so the oracle here is the reference
//! implementation, not another reimplementation of the specification.

pub use alloy_primitives::{Address, Bytes, FixedBytes, Signed, Uint};
pub use byteorder;
pub use bytes::BytesMut;
pub use fluentbase_codec::{FunctionArgs, SolidityABI};
pub use hex;

/// One failed check, kept as text so the whole run can be reported at once.
pub struct Divergence {
    pub case: String,
    pub direction: &'static str,
    pub detail: String,
}

#[derive(Default)]
pub struct Report {
    pub divergences: Vec<Divergence>,
    pub checked: usize,
}

impl Report {
    pub fn fail(&mut self, case: &str, direction: &'static str, detail: String) {
        self.divergences.push(Divergence {
            case: case.to_string(),
            direction,
            detail,
        });
    }

    /// Panics with the full list if anything diverged. A run that reports only its first failure
    /// hides how wide a problem is, which matters at 1880 vectors.
    pub fn assert_clean(&self, suite: &str) {
        if self.divergences.is_empty() {
            println!("{suite}: {} vectors, no divergences", self.checked);
            return;
        }

        let mut out = format!(
            "\n{suite}: {} of {} vectors diverged from solc\n\n",
            self.divergences.len(),
            self.checked
        );
        for d in self.divergences.iter().take(400) {
            out.push_str(&format!(
                "  [{}] {}\n      {}\n",
                d.direction, d.case, d.detail
            ));
        }
        if self.divergences.len() > 400 {
            out.push_str(&format!("  … and {} more\n", self.divergences.len() - 400));
        }
        panic!("{out}");
    }
}

// Value constructors the generated code calls. Emitting Rust literals instead would make the
// generated file an order of magnitude larger for no gain.

pub fn addr(value: &str) -> Address {
    value.parse().expect("corpus address")
}

pub fn b(value: &str) -> Bytes {
    Bytes::from(hex::decode(value.trim_start_matches("0x")).expect("corpus bytes"))
}

pub fn fb<const N: usize>(value: &str) -> FixedBytes<N> {
    FixedBytes::from_slice(
        &hex::decode(value.trim_start_matches("0x")).expect("corpus fixed bytes"),
    )
}

pub fn uint<const BITS: usize, const LIMBS: usize>(value: &str) -> Uint<BITS, LIMBS> {
    Uint::from_str_radix(value, 10).expect("corpus unsigned integer")
}

pub fn int<const BITS: usize, const LIMBS: usize>(value: &str) -> Signed<BITS, LIMBS> {
    Signed::from_dec_str(value).expect("corpus signed integer")
}

/// One corpus vector: its name, the value, and the bytes solc produced for it.
pub type Vector<'a, T> = (&'a str, T, &'a str);

/// Encodes each vector, compares against the bytes solc produced, then decodes back.
///
/// `FunctionArgs` is exported from `fluentbase-codec`, so this is an ordinary generic function.
pub fn check_vectors<T>(report: &mut Report, vectors: Vec<Vector<'_, T>>)
where
    T: FunctionArgs<byteorder::BE, 32, true, false> + PartialEq + core::fmt::Debug,
{
    for (name, value, expected) in vectors {
        report.checked += 1;

        let mut buf = BytesMut::new();
        if let Err(error) = SolidityABI::encode_function_args(&value, &mut buf) {
            report.fail(name, "encode", format!("{error:?}"));
            continue;
        }

        let ours = hex::encode(&buf[..]);
        if ours != expected {
            report.fail(
                name,
                "bytes",
                format!("ours = {ours}\n      solc = {expected}"),
            );
            continue;
        }

        let raw = buf.freeze();
        match SolidityABI::<T>::decode_function_args(&raw) {
            Ok(back) if back == value => {}
            Ok(back) => report.fail(name, "round trip", format!("decoded to {back:?}")),
            Err(error) => report.fail(name, "round trip", format!("rejected: {error:?}")),
        }
    }
}
