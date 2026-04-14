mod builtin;
mod ci_check;
mod cli;
mod config;
mod detect;
mod detect_patterns;
mod doctor;
mod eval;
mod generate_context;
mod generate_tests;
mod handoff;
mod layers;
mod output;
mod personal;
mod resolve;
mod rules;
mod state;
mod status;
mod team;
mod triggers;
mod types;
mod update;

pub fn run() -> i32 {
    cli::run()
}
