//! Provider access after resolver admission.
//!
//! This module deliberately does not own DNS, HTTP, redirect, or extraction
//! code. Slice 10 will connect this boundary to the final transport adapters.

mod authorized_adapter;

pub(crate) use authorized_adapter::*;
