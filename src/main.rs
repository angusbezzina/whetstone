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
mod output;
mod resolve;
mod rules;
mod state;
mod status;
mod types;
mod update;

fn main() {
    let exit_code = cli::run();
    std::process::exit(exit_code);
}
