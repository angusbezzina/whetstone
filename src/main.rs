mod ci_check;
mod cli;
mod config;
mod detect;
mod doctor;
mod output;
mod resolve;
mod state;
mod status;
mod types;

fn main() {
    let exit_code = cli::run();
    std::process::exit(exit_code);
}
