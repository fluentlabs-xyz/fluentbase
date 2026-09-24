//! Contains Rwasm specific precompiles.
use crate::RwasmSpecId;
use revm::{
    context::Cfg,
    context_interface::ContextTr,
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{CallInputs, InterpreterResult},
    precompile::{PrecompileSpecId, Precompiles},
    primitives::Address,
};
use std::{boxed::Box, string::String};

/// Rwasm precompile provider: the native `revm` implementations of the Ethereum precompiles.
///
/// The live networks were launched with an rWASM contract at every precompile address. Those
/// guests defined the chain's precompile behaviour and remain in state there (code hash,
/// `EXTCODESIZE`, `EXTCODEHASH` masking), but a call no longer reaches them: the frame consults
/// this provider before it loads the account code, and the host implementation returns the same
/// output and charges the same gas as the guest did. A fresh genesis installs nothing at those
/// addresses, and a runtime upgrade of one of them has no effect on execution.
#[derive(Debug, Clone)]
pub struct RwasmPrecompiles {
    /// Inner precompile provider is the same as Ethereum.
    inner: EthPrecompiles,
    /// Spec id of the precompiled provider.
    spec: RwasmSpecId,
}

impl RwasmPrecompiles {
    /// Create a new precompile provider with the given OpSpec.
    #[inline]
    pub fn new_with_spec(spec: RwasmSpecId) -> Self {
        let precompiles = native_precompiles(spec);
        Self {
            inner: EthPrecompiles { precompiles, spec },
            spec,
        }
    }

    /// Precompiles getter.
    #[inline]
    pub fn precompiles(&self) -> &'static Precompiles {
        self.inner.precompiles
    }
}

/// The native precompile set of `spec`, floored at Osaka.
///
/// The genesis guests implement exactly the Osaka set (`modexp` with the EIP-7883 pricing,
/// `P256VERIFY` at `0x100` with the Osaka fee, the EIP-2537 BLS12-381 operations) and have done so
/// at every block, before and after each chain's Osaka activation. A spec below Osaka therefore
/// maps to the Osaka set, otherwise a node replaying a pre-Osaka block would price `modexp` by the
/// Berlin rules the guest never applied. Later specs take their own set, so a future fork adds or
/// reprices precompiles through the spec like on Ethereum.
fn native_precompiles(spec: RwasmSpecId) -> &'static Precompiles {
    let spec = core::cmp::max(spec, RwasmSpecId::OSAKA);
    Precompiles::new(PrecompileSpecId::from_spec_id(spec))
}

impl<CTX> PrecompileProvider<CTX> for RwasmPrecompiles
where
    CTX: ContextTr<Cfg: Cfg<Spec = RwasmSpecId>>,
{
    type Output = InterpreterResult;

    #[inline]
    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        if spec == self.spec {
            return false;
        }
        *self = Self::new_with_spec(spec);
        true
    }

    #[inline]
    fn run(
        &mut self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        self.inner.run(context, inputs)
    }

    /// The canonical EIP-2929 warm set of the spec.
    ///
    /// Anything that mirrors the chain wraps the provider in [`ColdPrecompiles`] instead: no
    /// network has ever pre-warmed precompile addresses.
    #[inline]
    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        let precompiles = Precompiles::new(PrecompileSpecId::from_spec_id(self.spec));
        Box::new(precompiles.addresses().cloned())
    }

    #[inline]
    fn contains(&self, address: &Address) -> bool {
        self.inner.contains(address)
    }
}

impl Default for RwasmPrecompiles {
    fn default() -> Self {
        Self::new_with_spec(RwasmSpecId::PRAGUE)
    }
}

/// A precompile provider that pre-warms no address.
///
/// Fluent has never seeded the EIP-2929 warm set with the precompile addresses: the node used to
/// install an empty provider while the precompiles were genesis rWASM contracts, so the first call
/// to one in a transaction pays the cold account-access cost (see `docs/04-gas-and-fuel.md`).
/// Canonical receipts on every network reflect that, so a provider that runs the precompiles
/// natively must keep reporting an empty warm set. The node and the fixture replay wrap their
/// provider in this type; every other call is delegated to the wrapped provider.
#[derive(Debug, Clone, Default)]
pub struct ColdPrecompiles<P>(pub P);

impl<CTX, P> PrecompileProvider<CTX> for ColdPrecompiles<P>
where
    CTX: ContextTr,
    P: PrecompileProvider<CTX>,
{
    type Output = P::Output;

    #[inline]
    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        self.0.set_spec(spec)
    }

    #[inline]
    fn run(
        &mut self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        self.0.run(context, inputs)
    }

    #[inline]
    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        Box::new(core::iter::empty())
    }

    #[inline]
    fn contains(&self, address: &Address) -> bool {
        self.0.contains(address)
    }
}
