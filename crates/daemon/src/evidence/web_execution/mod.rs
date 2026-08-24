//! Provider access after resolver admission.
//!
//! This module deliberately does not own DNS, HTTP, redirect, or extraction
//! code. Slice 10 will connect this boundary to the final transport adapters.

mod authorized_adapter;

// Slice 10 connects this boundary to the final transport adapters. Until then
// the re-export deliberately has no in-crate consumer.
#[allow(unused_imports)]
pub(crate) use authorized_adapter::*;
