//! Wire-contract versioning for `TinyDocs` hosts.

/// Current `TinyDocs` wire-contract version.
///
/// Intake methods are additive to version 2. Older version-2 modules may not
/// serve them; hosts must check method availability or require a newer release.
pub const CONTRACT_VERSION: u32 = 2;

/// Returns whether a host requiring `required` can bind to this contract.
///
/// The first contract revision supports only exact-version bindings. A future
/// backward-compatible revision may widen this rule deliberately.
#[must_use]
pub const fn is_compatible(required: u32) -> bool {
    required == CONTRACT_VERSION
}
