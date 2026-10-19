pub mod act;
pub mod annotations;
mod ast;
pub mod catalogue;
mod cli;
pub mod compile;
pub mod contract;
pub mod dashboard;
pub mod dashboard_service;
pub mod domain;
pub mod execution;
pub mod gates;
pub mod layout;
mod output;
pub mod probe;
pub mod projection;
pub mod rulebook;
pub mod scripts;
pub mod service;
pub mod starter;
pub mod wire;

pub fn run() -> i32 {
    cli::run()
}
