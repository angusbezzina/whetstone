pub mod agreement;
mod ast;
pub mod beads;
mod check;
mod cli;
pub mod dashboard;
pub mod dashboard_service;
pub mod domain;
pub mod execution;
pub mod feature_map;
pub mod gates;
pub mod governance;
pub mod history;
pub mod hygiene;
pub mod onboarding;
mod output;
pub mod policy;
pub mod projection;
pub mod repair_host;
pub mod repair_transport;
mod rules;
pub mod service;
pub mod skill;
pub mod storage;
pub mod sync;
mod types;
pub mod verification;

pub fn run() -> i32 {
    cli::run()
}
