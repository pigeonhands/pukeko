use std::sync::Arc;

use ratatui::layout::Rect;
use russh::keys::ssh_key::{self};
use russh::{Channel, ChannelId, MethodSet, Pty, SshId, server::*};
use tokio::sync::Mutex;
use tracing::{debug, error, info, trace, warn};

use crate::config::PukekoConfig;
use crate::tui::{MenuState, PukekoMenu, SshTerminal};

#[derive(Clone)]
pub struct PukekoServer {
    id: usize,
    config: Arc<PukekoConfig>,
}

impl PukekoServer {
    pub fn new(config: PukekoConfig) -> Self {
        Self {
            id: 0,
            config: Arc::new(config),
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let methods = {
            let mut ms = MethodSet::empty();
            ms.push(russh::MethodKind::PublicKey);
            ms
        };

        let config = Config {
            server_id: SshId::Standard(format!(
                "SSH-2.0-{}_{}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            )),
            inactivity_timeout: Some(std::time::Duration::from_secs(3600)),
            auth_rejection_time: std::time::Duration::from_millis(100),
            auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
            keys: vec![self.config.server_key.clone()],
            nodelay: true,
            methods,
            ..Default::default()
        };
        self.run_on_address(Arc::new(config), ("0.0.0.0", 2222))
            .await?;
        Ok(())
    }
}

impl Server for PukekoServer {
    type Handler = ClientConnection;
    fn new_client(&mut self, saddr: Option<std::net::SocketAddr>) -> Self::Handler {
        self.id += 1;

        debug!("{}] Got connection from {:?}", self.id, saddr);
        ClientConnection::new(self.config.clone(), self.id)
    }

    fn handle_session_error(&mut self, error: <Self::Handler as Handler>::Error) {
        error!("Session error: {:?}", error);
    }
}

pub enum ConnectionState {
    Connected,
    AtMenu {
        terminal: SshTerminal,
        menu: PukekoMenu,
    },
    //Forwarding,
}

pub struct ClientConnection {
    config: Arc<PukekoConfig>,
    connection_state: ConnectionState,
    id: usize,
}

impl ClientConnection {
    pub fn new(config: Arc<PukekoConfig>, id: usize) -> Self {
        Self {
            config,
            connection_state: ConnectionState::Connected,
            id,
        }
    }
}

impl Handler for ClientConnection {
    type Error = anyhow::Error;

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        public_key: &ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        if public_key == &self.config.user_key {
            trace!(
                "{}] Accepting {} offered ssh public key {:?}",
                self.id,
                user,
                public_key.to_openssh()?
            );
            Ok(Auth::Accept)
        } else {
            trace!(
                "{}] Rejecting {} offered ssh public key {:?}",
                self.id,
                user,
                public_key.to_openssh()?
            );
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        trace!(
            "{}] User {} requested auth with public key {:?}",
            self.id,
            user,
            public_key.to_openssh()?
        );
        info!(
            "{}] Accepting user {} auth pubkey {:?}",
            self.id,
            user,
            public_key.to_openssh()?
        );
        Ok(Auth::Accept)
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        match &mut self.connection_state {
            ConnectionState::AtMenu { terminal, menu } => {
                menu.handle_data(data).await?;
                terminal.render(menu)?;

                match menu.state() {
                    MenuState::Closing => {
                        session.close(channel)?;
                    }
                    _ => {}
                }
            }
            _ => {
                warn!("{}] Got data without a menu open", self.id);
            }
        };
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _: ChannelId,
        col_width: u32,
        row_height: u32,
        _: u32,
        _: u32,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        let rect = Rect {
            x: 0,
            y: 0,
            width: col_width as u16,
            height: row_height as u16,
        };

        match &mut self.connection_state {
            ConnectionState::AtMenu { terminal, menu } => {
                trace!("{}] trying to resize menu...", self.id);
                terminal.resize(rect)?;
                terminal.render(menu)?;
            }
            _ => {
                warn!("{}] Got data without a menu open", self.id);
            }
        };

        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        col_width: u32,
        row_height: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let rect = Rect {
            x: 0,
            y: 0,
            width: col_width as u16,
            height: row_height as u16,
        };

        match &mut self.connection_state {
            ConnectionState::AtMenu { terminal, menu } => {
                trace!("{}] creating pseudo terminal", self.id);
                terminal.resize(rect)?;
                terminal.render(menu)?;

                session.channel_success(channel)?;
            }
            _ => {
                warn!(
                    "{}] Attempted to create a pseudo terminal without a terminal handle",
                    self.id
                );
            }
        };

        Ok(())
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        if matches!(self.connection_state, ConnectionState::Connected) {
            let (terminal, menu) = PukekoMenu::from_session(channel, session).await?;
            self.connection_state = ConnectionState::AtMenu { terminal, menu };
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> anyhow::Result<()> {
        session.close(channel)?;
        info!("{}] disconnected", self.id);
        Ok(())
    }
}
