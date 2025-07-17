use russh::keys::{PrivateKey, PublicKey};

#[derive(Debug, Clone)]
pub struct PukekoConfig {
    pub server_key: PrivateKey,

    pub user_key: PublicKey,
}
