#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::task::production) struct BundleSummary {
    pub(in crate::task::production) output_bytes: usize,
    pub(in crate::task::production) output_digest: String,
    pub(in crate::task::production) module_count: usize,
}
