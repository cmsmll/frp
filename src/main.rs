use std::io;

use clap::Parser;
use frp::{Config, client::Client, server::Proxy};

use tikv_jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() {
    if let Err(err) = Cli::parse().run().await {
        eprintln!("{err}")
    }
}

#[derive(Parser)]
enum Cli {
    Client(Arg),
    Server(Arg),
}

#[derive(Parser)]
struct Arg {
    #[arg(default_value = "config.toml")]
    path: String,
}

impl Cli {
    async fn run(self) -> io::Result<()> {
        match self {
            Cli::Client(arg) => {
                let config = Config::load(&arg.path)?;
                let client = Client::new(config);
                client.run().await?;
            }
            Cli::Server(arg) => {
                let config = Config::load(&arg.path)?;
                Proxy::run(config.into()).await?;
            }
        };
        Ok(())
    }
}
