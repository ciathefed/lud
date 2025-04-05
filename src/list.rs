use std::io;

use anyhow::{Result, anyhow};
use crossterm::event::KeyCode;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{event, execute};
use tui::backend::CrosstermBackend;
use tui::widgets::ListState;
use tui::{
    Terminal,
    widgets::{Block, Borders, List, ListItem},
};

use crate::settings::Server;

pub struct StatefulList<'a> {
    state: ListState,
    items: Vec<&'a Server>,
}

impl<'a> StatefulList<'a> {
    fn new(items: Vec<&'a Server>) -> StatefulList<'a> {
        StatefulList {
            state: ListState::default(),
            items,
        }
    }

    fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }
}

pub fn select_server_from_list(servers: &[Server]) -> Result<&Server> {
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    enable_raw_mode()?;
    execute!(stdout, Clear(ClearType::All))?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut stateful_list = StatefulList::new(servers.iter().collect());
    stateful_list.state.select(Some(0));

    let result = loop {
        terminal.draw(|f| {
            let size = f.size();
            let block = Block::default()
                .borders(Borders::ALL)
                .title("Select Server (↑/↓ to navigate, Enter to select, Esc to quit)");
            f.render_widget(block, size);

            let items: Vec<ListItem> = stateful_list
                .items
                .iter()
                .map(|server| ListItem::new(server.name.clone()))
                .collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL))
                .highlight_symbol(">> ");

            f.render_stateful_widget(list, size, &mut stateful_list.state);
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let event::Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Esc => {
                        break Err(anyhow!("Selection cancelled"));
                    }
                    KeyCode::Up => stateful_list.previous(),
                    KeyCode::Down => stateful_list.next(),
                    KeyCode::Enter => {
                        if let Some(selected) = stateful_list.state.selected() {
                            break Ok(stateful_list.items[selected]);
                        }
                    }
                    _ => {}
                }
            }
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}
