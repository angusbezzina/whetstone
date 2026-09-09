mod ast;
mod check;
mod cli;
pub mod domain;
mod output;
mod rules;
pub mod storage;
mod types;

pub fn run() -> i32 {
    cli::run()
}
