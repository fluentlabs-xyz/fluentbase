use super::{Index, Store, StoreContext, Stored};
use crate::ValueType;
use alloc::{sync::Arc, vec::Vec};

/// A raw index to a function signature entity.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SignatureIdx(usize);

impl Index for SignatureIdx {
    fn into_usize(self) -> usize {
        self.0
    }

    fn from_usize(value: usize) -> Self {
        Self(value)
    }
}

/// A function signature containing the inputs and outputs.
///
/// # Note
///
/// The inputs and outputs are ordered and merged in a single
/// vector starting with by inputs by their order and following
/// up with the outputs.
/// The `len_inputs` field denotes how many inputs there are in
/// the head of the vector.
#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
pub struct SignatureEntity {
    /// The ordered and merged inputs and outputs of the function signature.
    inputs_outputs: Arc<[ValueType]>,
    /// The number of inputs.
    len_inputs: usize,
}

impl SignatureEntity {
    /// Creates a new function signature.
    pub fn new<I, O>(inputs: I, outputs: O) -> Self
    where
        I: IntoIterator<Item = ValueType>,
        O: IntoIterator<Item = ValueType>,
        I::IntoIter: ExactSizeIterator,
    {
        let inputs = inputs.into_iter();
        let len_inputs = inputs.len();
        let inputs_outputs = Arc::from(inputs.chain(outputs).collect::<Vec<_>>());
        Self {
            inputs_outputs,
            len_inputs,
        }
    }

    /// Returns the inputs of the function signature.
    pub fn inputs(&self) -> &[ValueType] {
        &self.inputs_outputs[..self.len_inputs]
    }

    /// Returns the outputs of the function signature.
    pub fn outputs(&self) -> &[ValueType] {
        &self.inputs_outputs[self.len_inputs..]
    }

    /// Returns the pair of inputs and outputs of the function signature.
    pub fn inputs_outputs(&self) -> (&[ValueType], &[ValueType]) {
        self.inputs_outputs.split_at(self.len_inputs)
    }
}

/// A Wasm function signature reference.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct Signature(Stored<SignatureIdx>);

impl Signature {
    /// Creates a new function signature reference.
    pub(super) fn from_inner(stored: Stored<SignatureIdx>) -> Self {
        Self(stored)
    }

    /// Returns the underlying stored representation.
    pub(super) fn into_inner(self) -> Stored<SignatureIdx> {
        self.0
    }

    /// Creates a new function signature to the store.
    pub fn new<T, I, O>(ctx: &mut Store<T>, inputs: I, outputs: O) -> Self
    where
        I: IntoIterator<Item = ValueType>,
        O: IntoIterator<Item = ValueType>,
        I::IntoIter: ExactSizeIterator,
    {
        ctx.alloc_signature(SignatureEntity::new(inputs, outputs))
    }

    /// Returns the inputs of the function signature.
    ///
    /// # Panics
    ///
    /// Panics if `ctx` does not own this [`Signature`].
    pub fn inputs<'a, T: 'a>(&self, ctx: impl Into<StoreContext<'a, T>>) -> &'a [ValueType] {
        ctx.into().store.resolve_signature(*self).inputs()
    }

    /// Returns the outputs of the function signature.
    ///
    /// # Panics
    ///
    /// Panics if `ctx` does not own this [`Signature`].
    pub fn outputs<'a, T: 'a>(&self, ctx: impl Into<StoreContext<'a, T>>) -> &'a [ValueType] {
        ctx.into().store.resolve_signature(*self).outputs()
    }

    /// Returns the outputs of the function signature.
    ///
    /// # Panics
    ///
    /// Panics if `ctx` does not own this [`Signature`].
    pub fn inputs_outputs<'a, T: 'a>(
        &self,
        ctx: impl Into<StoreContext<'a, T>>,
    ) -> (&'a [ValueType], &'a [ValueType]) {
        ctx.into().store.resolve_signature(*self).inputs_outputs()
    }
}
