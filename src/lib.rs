mod adherence;
mod approve;
mod ast;
mod check;
mod ci_check;
mod cli;
mod config;
mod config_packs;
mod debt;
mod detect;
mod doctor;
mod extract;
mod gen;
mod generate_context;
mod generate_lint;
mod generate_tests;
mod handoff;
mod layers;
mod output;
mod personal;
mod report;
mod resolve;
mod review;
mod rule_authoring;
mod rules;
mod rules_query;
mod second_brain;
mod source_mgmt;
mod state;
mod status;
mod templates;
mod triggers;
mod tui;
mod types;
mod update;
mod worklist;

pub fn run() -> i32 {
    cli::run()
}
