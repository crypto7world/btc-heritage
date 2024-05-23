mod commands;

use clap::Parser;
use commands::CliOpts;

fn main() {
    env_logger::init();

    let cli_opts = CliOpts::parse();
    log::debug!("Processing {:?}", cli_opts);
}
