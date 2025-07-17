use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use russh::server::Session;

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};
use russh::server::*;
use russh::{Channel, ChannelId};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tracing::trace;

pub struct SshTerminal(Terminal<CrosstermBackend<TerminalHandle>>);

impl SshTerminal {
    pub async fn new(channel: Channel<Msg>, session: &mut Session) -> anyhow::Result<Self> {
        let terminal_handle = TerminalHandle::start(session.handle(), channel.id()).await;

        let backend = CrosstermBackend::new(terminal_handle);

        let options = TerminalOptions {
            viewport: Viewport::Fixed(Rect::default()),
        };
        Ok(Self(Terminal::with_options(backend, options)?))
    }

    pub fn render(&mut self, menu: &mut PukekoMenu) -> anyhow::Result<()> {
        if matches!(menu.state(), MenuState::Open) {
            self.0.draw(|frame| menu.render_menu(frame))?;
        } else {
            self.0
                .draw(|frame| frame.render_widget(Clear, frame.area()))?;
        }
        Ok(())
    }

    pub fn resize(&mut self, area: Rect) -> anyhow::Result<()> {
        self.0.resize(area)?;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy)]
pub enum MenuState {
    Open,
    Closing,
}

struct UI {
    list_state: ListState,
}

pub struct PukekoMenu {
    parser: termwiz::escape::parser::Parser,

    items: Vec<String>,
    ui: UI,
    state: MenuState,
}

impl PukekoMenu {
    pub async fn from_session(
        channel: Channel<Msg>,
        session: &mut Session,
    ) -> anyhow::Result<(SshTerminal, Self)> {
        let terminal = SshTerminal::new(channel, session).await?;

        Ok((
            terminal,
            Self {
                parser: termwiz::escape::parser::Parser::new(),
                items: vec!["Hello".into(), "World".into(), "memes".into()],
                ui: UI {
                    list_state: ListState::default().with_selected(Some(0)),
                },
                state: MenuState::Open,
            },
        ))
    }

    pub fn state(&self) -> &MenuState {
        &self.state
    }

    fn render_menu(&mut self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Clear, area);

        let paragraph = Paragraph::new(format!("Counter: "))
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(Color::Green));

        let block = Block::default()
            .title("Press 'q' to quit")
            .borders(Borders::ALL);

        let vertical_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Percentage(25),
                    Constraint::Fill(1),
                    Constraint::Percentage(25),
                ]
                .as_ref(),
            )
            .split(block.inner(area));

        let middle_vertical_chunk = vertical_chunks[1];

        let horizontal_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Fill(1),
                Constraint::Percentage(20),
            ])
            .split(middle_vertical_chunk);

        let center_block = horizontal_chunks[1];

        let items: Vec<ListItem> = self
            .items
            .iter()
            .map(|i| ListItem::new(Line::from(i.clone())))
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Select Server"),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::LightGreen)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        f.render_widget(paragraph.block(block), area);
        f.render_stateful_widget(list, center_block, &mut self.ui.list_state);
    }

    fn select_item_down(&mut self) {
        let ui = &mut self.ui;
        let i = if let Some(current_selected) = ui.list_state.selected() {
            if current_selected >= self.items.len() - 1 {
                0
            } else {
                current_selected + 1
            }
        } else {
            0
        };
        ui.list_state.select(Some(i));
    }

    fn select_item_up(&mut self) {
        let ui = &mut self.ui;
        let i = if let Some(current_selected) = ui.list_state.selected() {
            if current_selected == 0 {
                self.items.len()
            } else {
                current_selected - 1
            }
        } else {
            0
        };
        ui.list_state.select(Some(i));
    }

    pub async fn handle_data(&mut self, data: &[u8]) -> anyhow::Result<()> {
        use termwiz::escape::{
            Action,
            csi::{CSI, Cursor},
        };

        let mut data = data;
        while let Some((action, bytes_consumed)) = self.parser.parse_first(data) {
            data = &data[bytes_consumed..];

            match action {
                Action::Print('q') => {
                    self.state = MenuState::Closing;
                }
                Action::CSI(CSI::Cursor(Cursor::Up(_))) | Action::Print('k') => {
                    self.select_item_up();
                }
                Action::CSI(CSI::Cursor(Cursor::Down(_))) | Action::Print('j') => {
                    self.select_item_down();
                }
                _ => {}
            }

            trace!("Ansi code {:?}", action);
        }

        Ok(())
    }
}

struct TerminalHandle {
    sender: UnboundedSender<Vec<u8>>,
    sink: Vec<u8>,
}

impl TerminalHandle {
    async fn start(handle: Handle, channel_id: ChannelId) -> Self {
        let (sender, mut receiver) = unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            while let Some(data) = receiver.recv().await {
                let result = handle.data(channel_id, data.into()).await;
                if result.is_err() {
                    break;
                }
            }
        });
        Self {
            sender,
            sink: Vec::new(),
        }
    }
}

// The crossterm backend writes to the terminal handle.
impl std::io::Write for TerminalHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sink.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let result = self.sender.send(self.sink.clone());
        if result.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                result.unwrap_err(),
            ));
        }

        self.sink.clear();
        Ok(())
    }
}
