use clap::Parser;
use server::start;

mod classnames;
mod server;
mod source;

#[derive(Parser, Debug)]
#[command(author, version, about)]
/// A Language Server for css class names in markup, for web frontend projects.
struct Cli {
    /// Set log level, one of trace, debug, info, warn, error
    #[arg(short, long, default_value = "info", global = true)]
    level: tracing::Level,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .pretty()
        .with_max_level(cli.level)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();

    start().await
}
