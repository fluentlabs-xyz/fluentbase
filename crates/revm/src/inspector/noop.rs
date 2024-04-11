use crate::Inspector;
use fluentbase_types::IJournaledTrie;
/// Dummy [Inspector], helpful as standalone replacement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoOpInspector;

impl<DB: IJournaledTrie> Inspector<DB> for NoOpInspector {}
