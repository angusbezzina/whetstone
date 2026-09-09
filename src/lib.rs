mod ast;
mod check;
mod cli;
pub mod domain;
pub mod execution;
pub mod governance;
mod output;
pub mod policy;
mod rules;
pub mod service;
pub mod storage;
mod types;

pub fn run() -> i32 {
    cli::run()
}
