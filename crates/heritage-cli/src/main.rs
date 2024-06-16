mod commands;

use clap::Parser;
use commands::CliParser;

fn main() {
    env_logger::init();

    let cli_opts = CliParser::parse();
    log::debug!("Processing {:?}", cli_opts);
}
