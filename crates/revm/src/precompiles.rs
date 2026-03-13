//! Contains Rwasm specific precompiles.
use crate::RwasmSpecId;
use once_cell::race::OnceBox;
use revm::{
    context::Cfg,
    context_interface::ContextTr,
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{CallInputs, InterpreterResult},
    precompile::{PrecompileSpecId, Precompiles},
    primitives::Address,
};
use std::{boxed::Box, string::String};

/// Rwasm precompile provider
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
        let precompiles = empty_precompile_list();
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

/// Returns precompiles for Homestead spec.
fn empty_precompile_list() -> &'static Precompiles {
    static INSTANCE: OnceBox<Precompiles> = OnceBox::new();
    INSTANCE.get_or_init(|| {
        let precompiles = Precompiles::default();
        // there are no any precompiled contracts at genesis level,
        // they're injected directly in the genesis file as WebAssembly contracts
        Box::new(precompiles)
    })
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
