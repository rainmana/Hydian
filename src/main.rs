use clap::Parser;
use hydian::{app, cli::Cli, output::Printer};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let printer = Printer::from_cli(&cli);
    let debug = cli.debug;

    if let Err(error) = app::run(cli).await {
        printer.failure("hydian", &error, debug);
        std::process::exit(1);
    }
}
