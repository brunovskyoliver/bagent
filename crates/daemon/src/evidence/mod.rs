#![allow(dead_code)]

#[cfg(feature = "stage8-acceptance")]
mod acceptance;
mod adapters;
mod classifier;
mod diagnostics;
mod events;
mod orchestrator;
mod planner;
mod runtime_control;
mod synthesis;
mod types;
mod validator;
mod web;
mod web_execution;

#[cfg(feature = "stage8-acceptance")]
pub(crate) use acceptance::*;
pub(crate) use adapters::*;
pub(crate) use classifier::*;
pub(crate) use diagnostics::*;
pub(crate) use events::*;
pub(crate) use orchestrator::*;
pub(crate) use planner::*;
pub(crate) use synthesis::*;
pub(crate) use types::*;
pub(crate) use validator::*;
pub(crate) use web::*;
pub(crate) use web_execution::*;

#[cfg(test)]
pub(crate) mod fixtures;

#[cfg(test)]
mod contract_tests;
