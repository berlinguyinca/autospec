//! Planning support for parallel decomposition.
//!
//! [`capacity`] resolves the effective implementation fleet capacity from the
//! five sources defined by the fleet-saturation spec (§6.2) and derives the
//! target initial wave width (§7.1). One resolver is reused by every caller.

pub mod capacity;
