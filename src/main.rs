mod config;
mod ssh;
mod tui;

use std::fs::File;
use std::io::Read;

use config::PukekoConfig;
use russh::keys::{PrivateKey, PublicKey};
use ssh::PukekoServer;

async fn start_server(config: PukekoConfig) -> anyhow::Result<()> {
    let mut server = PukekoServer::new(config);
    server.run().await.expect("Failed running server");

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing::Level::TRACE.into())
                .from_env_lossy(),
        )
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let server_key = {
        let mut buffer = Vec::new();
        let mut bytes = File::open("./test_data/keys/server_key")?;
        bytes.read_to_end(&mut buffer)?;

        PrivateKey::from_openssh(buffer)?
    };

    let config = PukekoConfig {
        server_key,
        user_key: PublicKey::from_openssh(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAcvtaYueykiTr1naUH2LrQcQ/R2/U8iPDQpEwTmDCpM",
        )?,
    };

    start_server(config).await
}
