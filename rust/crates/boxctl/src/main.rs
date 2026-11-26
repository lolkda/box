mod cli;
mod config;
mod core;
mod daemon;
mod doctor;
mod download;
mod error;
mod events;
mod geo;
mod install;
mod lists;
mod logs;
mod network;
mod network_rules;
mod paths;
mod peer;
mod protocol;
mod rule_profile;
mod schedule;
mod service;
mod settings;
mod sub;
mod task;
mod tools;
mod uninstall;
mod upgrade;
mod util;
mod yaml;

fn main() {
    let code = cli::run(std::env::args().skip(1).collect());
    std::process::exit(code);
}
