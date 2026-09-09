mod ast;
mod check;
mod cli;
mod output;
mod rules;
mod types;

pub fn run() -> i32 {
    cli::run()
}
