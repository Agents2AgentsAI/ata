use clap::Parser;
use codex_coordination_relay::RelayServerConfig;

#[derive(Parser)]
#[command(
    name = "codex-relay",
    about = "Coordination relay server for codex agents"
)]
struct Cli {
    /// Port to listen on.
    #[arg(long, default_value_t = 7800)]
    port: u16,

    /// Optional shared secret. When set, clients must send a matching
    /// `X-Relay-Secret` header on every request (except /health).
    #[arg(long)]
    secret: Option<String>,

    /// Maximum number of messages retained per project (ring buffer).
    #[arg(long, default_value_t = 100)]
    max_messages: usize,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    if let Err(e) = codex_coordination_relay::run_server(RelayServerConfig {
        port: cli.port,
        secret: cli.secret,
        max_messages: cli.max_messages,
    })
    .await
    {
        eprintln!("relay server error: {e}");
        std::process::exit(1);
    }
}
