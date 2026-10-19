mod ast;
pub mod catalogue;
mod cli;
pub mod dashboard;
pub mod dashboard_service;
pub mod domain;
pub mod execution;
pub mod gates;
pub mod layout;
mod output;
pub mod projection;
pub mod rulebook;
pub mod service;
pub mod starter;
pub mod wire;

pub fn run() -> i32 {
    cli::run()
}
