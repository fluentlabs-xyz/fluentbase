use super::GlobalIdx;
use crate::{errors::ModuleError, ExternType, Module};
use alloc::{boxed::Box, collections::btree_map::Iter as BTreeIter};

/// The index of a function declaration within a [`Module`].
///
/// [`Module`]: [`super::Module`]
#[derive(Debug, Copy, Clone)]
pub struct FuncIdx(u32);

impl From<u32> for FuncIdx {
    fn from(index: u32) -> Self {
        Self(index)
    }
}

impl FuncIdx {
    /// Returns the [`FuncIdx`] as `u32`.
    pub fn into_u32(self) -> u32 {
        self.0
    }
}

/// The index of a table declaration within a [`Module`].
///
/// [`Module`]: [`super::Module`]
#[derive(Debug, Copy, Clone)]
pub struct TableIdx(u32);

impl From<u32> for TableIdx {
    fn from(index: u32) -> Self {
        Self(index)
    }
}

impl TableIdx {
    /// Returns the [`TableIdx`] as `u32`.
    pub fn into_u32(self) -> u32 {
        self.0
    }
}

/// The index of a linear memory declaration within a [`Module`].
///
/// [`Module`]: [`super::Module`]
#[derive(Debug, Copy, Clone)]
pub struct MemoryIdx(u32);

impl From<u32> for MemoryIdx {
    fn from(index: u32) -> Self {
        Self(index)
    }
}

impl MemoryIdx {
    /// Returns the [`MemoryIdx`] as `u32`.
    pub fn into_u32(self) -> u32 {
        self.0
    }
}

/// An external item of an [`ExportType`] definition within a [`Module`].
///
/// [`Module`]: [`crate::Module`]
#[derive(Debug, Copy, Clone)]
pub enum ExternIdx {
    /// An exported function and its index within the [`Module`].
    ///
    /// [`Module`]: [`super::Module`]
    Func(FuncIdx),
    /// An exported table and its index within the [`Module`].
    ///
    /// [`Module`]: [`super::Module`]
    Table(TableIdx),
    /// An exported linear memory and its index within the [`Module`].
    ///
    /// [`Module`]: [`super::Module`]
    Memory(MemoryIdx),
    /// An exported global variable and its index within the [`Module`].
    ///
    /// [`Module`]: [`super::Module`]
    Global(GlobalIdx),
}

impl From<FuncIdx> for ExternIdx {
    fn from(value: FuncIdx) -> Self {
        Self::Func(value)
    }
}
impl From<TableIdx> for ExternIdx {
    fn from(value: TableIdx) -> Self {
        Self::Table(value)
    }
}
impl From<MemoryIdx> for ExternIdx {
    fn from(value: MemoryIdx) -> Self {
        Self::Memory(value)
    }
}
impl From<GlobalIdx> for ExternIdx {
    fn from(value: GlobalIdx) -> Self {
        Self::Global(value)
    }
}

impl ExternIdx {
    /// Create a new [`ExternIdx`] from the given [`wasmparser::ExternalKind`] and `index`.
    ///
    /// # Errors
    ///
    /// If an unsupported external definition is encountered.
    pub fn new(kind: wasmparser::ExternalKind, index: u32) -> Result<Self, ModuleError> {
        match kind {
            wasmparser::ExternalKind::Func => Ok(ExternIdx::Func(FuncIdx(index))),
            wasmparser::ExternalKind::Table => Ok(ExternIdx::Table(TableIdx(index))),
            wasmparser::ExternalKind::Memory => Ok(ExternIdx::Memory(MemoryIdx(index))),
            wasmparser::ExternalKind::Global => Ok(ExternIdx::Global(GlobalIdx::from(index))),
            wasmparser::ExternalKind::Tag => {
                panic!("wasmi does not support the `exception-handling` Wasm proposal")
            }
        }
    }

    pub fn into_func_idx(self) -> Option<u32> {
        match self {
            ExternIdx::Func(idx) => Some(idx.into_u32()),
            _ => None,
        }
    }
}

/// An iterator over the exports of a [`Module`].
///
/// [`Module`]: [`super::Module`]
#[derive(Debug)]
pub struct ModuleExportsIter<'module> {
    exports: BTreeIter<'module, Box<str>, ExternIdx>,
    module: &'module Module,
}

/// A descriptor for an exported WebAssembly value of a [`Module`].
///
/// This type is primarily accessed from the [`Module::exports`] method and describes
/// what names are exported from a Wasm [`Module`] and the type of the item that is exported.
#[derive(Debug)]
pub struct ExportType<'module> {
    name: &'module str,
    ty: ExternType,
    index: ExternIdx,
}

impl<'module> ExportType<'module> {
    /// Returns the name by which the export is known.
    pub fn name(&self) -> &'module str {
        self.name
    }

    /// Returns the type of the exported item.
    pub fn ty(&self) -> &ExternType {
        &self.ty
    }

    pub fn index(&self) -> &ExternIdx {
        &self.index
    }
}

impl<'module> ModuleExportsIter<'module> {
    /// Creates a new [`ModuleExportsIter`] from the given [`Module`].
    pub(super) fn new(module: &'module Module) -> Self {
        Self {
            exports: module.exports.iter(),
            module,
        }
    }
}

impl<'module> Iterator for ModuleExportsIter<'module> {
    type Item = ExportType<'module>;

    fn next(&mut self) -> Option<Self::Item> {
        self.exports.next().map(|(name, idx)| {
            let ty = self.module.get_extern_type(*idx);
            ExportType {
                name,
                ty,
                index: *idx,
            }
        })
    }
}
