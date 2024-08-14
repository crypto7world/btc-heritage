mod commands;
mod display;
mod spendflow;
mod utils;

use clap::Parser;
use commands::CliParser;

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("error,tracing::span=warn"),
    )
    .format_timestamp_micros()
    .init();

    let cli_parser = CliParser::parse();
    log::debug!("Processing {:?}", cli_parser);
    match cli_parser.execute() {
        Ok(displayable) => displayable.display(),
        Err(e) => log::error!("{e}"),
    };
}
