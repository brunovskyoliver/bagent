use anyhow::Result;
use std::{future::Future, pin::Pin};

pub type InferenceFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

/// Narrow model-inference interface used by classifier modules. Runtime
/// residency, leases, lifecycle, and transport remain behind the daemon's
/// Model Runtime adapter.
pub trait AgentInference: Send + Sync {
    fn infer_raw<'a>(
        &'a self,
        model: &'a str,
        prompt: &'a str,
        temperature: f32,
    ) -> InferenceFuture<'a>;

    fn infer_json<'a>(
        &'a self,
        model: &'a str,
        prompt: &'a str,
        temperature: f32,
    ) -> InferenceFuture<'a>;
}
