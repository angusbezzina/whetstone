pub mod agreement;
mod ast;
pub mod beads;
pub mod catalogue;
mod check;
mod cli;
pub mod dashboard;
pub mod dashboard_service;
pub mod domain;
pub mod eval;
pub mod execution;
pub mod exemplar;
pub mod feature_map;
pub mod gates;
pub mod hands;
pub mod history;
pub mod hosts;
pub mod hygiene;
pub mod judgment;
pub mod learning;
pub mod onboarding;
mod output;
pub mod projection;
pub mod proof;
pub(crate) mod rules;
pub mod service;
pub mod setup;
pub mod skill;
pub mod starter;
pub mod storage;
pub mod sync;
mod types;
pub mod verification;

pub fn run() -> i32 {
    cli::run()
}
