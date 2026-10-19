pub mod agreement;
mod ast;
pub mod beads;
pub mod catalogue;
mod cli;
pub mod dashboard;
pub mod dashboard_service;
pub mod domain;
pub mod execution;
pub mod gates;
pub mod history;
mod output;
pub mod projection;
pub mod rulebook;
pub mod service;
pub mod setup;
pub mod starter;
pub mod storage;
pub mod store;
pub mod sync;
pub mod wire;

pub fn run() -> i32 {
    cli::run()
}
