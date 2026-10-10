//! Wire-contract versioning for `TinyDocs` hosts.

/// Current `TinyDocs` wire-contract version.
///
/// Version 3 adds full Markdown conversion. Older modules retain their existing
/// member signatures but do not serve `ConvertMarkdown`; hosts requiring it
/// must pin a compatible released artifact.
pub const CONTRACT_VERSION: u32 = 3;

/// Returns whether a host requiring `required` can bind to this contract.
///
/// The first contract revision supports only exact-version bindings. A future
/// backward-compatible revision may widen this rule deliberately.
#[must_use]
pub const fn is_compatible(required: u32) -> bool {
    required == CONTRACT_VERSION
}
