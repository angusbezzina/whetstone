mod ast;
mod check;
mod cli;
pub mod dashboard;
pub mod dashboard_service;
pub mod domain;
pub mod execution;
pub mod governance;
pub mod history;
pub mod onboarding;
mod output;
pub mod policy;
pub mod repair;
mod rules;
pub mod service;
pub mod storage;
mod types;
pub mod verification;

pub fn run() -> i32 {
    cli::run()
}
