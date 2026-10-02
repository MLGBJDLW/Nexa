//! Compatibility export for the shared model catalog protocol.
//!
//! The implementation is an independent crate: catalog identities, capability
//! merging and immutable data lookup cannot depend on the database, agent,
//! browser, capture, or desktop host.
pub use nexa_model_catalog::*;
