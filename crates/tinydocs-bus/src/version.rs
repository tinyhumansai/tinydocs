//! Wire-contract versioning for `TinyDocs` hosts.

/// Current `TinyDocs` wire-contract version.
///
/// Version 4 adds bounded image inspection. Hosts requiring this operation
/// must pin a compatible released artifact.
pub const CONTRACT_VERSION: u32 = 4;

/// Returns whether a host requiring `required` can bind to this contract.
///
/// The first contract revision supports only exact-version bindings. A future
/// backward-compatible revision may widen this rule deliberately.
#[must_use]
pub const fn is_compatible(required: u32) -> bool {
    required == CONTRACT_VERSION
}
