use std::{
    cell::RefCell,
    io::{self, BufReader},
    fs::File,
    rc::Rc
};
use vctools_utils::preamble::*;

use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::Stylize,
    widgets::{
        Block, Borders, BorderType, Paragraph, StatefulWidget, Widget
    },
    DefaultTerminal
};
use directories::ProjectDirs;
use serde::Deserialize;

mod action;
mod msgbox;
mod topwidget;

use action::{ActionBar, ActionBarMode, ActionBarState, Commands, CommandsMap, Response};
use msgbox::MessageBox;
use topwidget::TopWidget;

#[derive(Debug, Deserialize)]
struct Account {
    name: String,
    kind: String,
    url: String,
    user: String,
    token: String,
}

#[derive(Debug, Default, Deserialize)]
struct Settings {
    accounts: Vec<Account>,
}

#[derive(Debug)]
struct AppState {
    exit: bool,
    action_bar: ActionBarState,
    terminal: Option<Rc<RefCell<DefaultTerminal>>>,
}

#[derive(Debug)]
struct App {
    settings: Settings,
    project_dirs: ProjectDirs,
    commands: Rc<Commands>,
    commands_map: CommandsMap<for<'a, 'b> fn(&mut App)>,
    action_bar: ActionBar,
    state: AppState,
}

impl App {
    pub fn init() -> io::Result<Self> {
        let project_dirs = ProjectDirs::from("", "", "git-review-tui").unwrap();

        std::fs::create_dir_all(&project_dirs.config_dir())?;
        std::fs::create_dir_all(&project_dirs.cache_dir())?;

        let mut commands = Commands::new();
        let mut commands_map: CommandsMap<for<'a, 'b> fn(&mut App)> = CommandsMap::new();

        commands_map.set(commands.add_command("quit", &["Quit", "Exit"]), App::cmd_quit);
        commands_map.set(commands.add_command("account-add", &["Add Account"]), App::cmd_account_add);

        commands.add_command("foo", &["Foo"]);
        commands.add_command("bar", &["Bar"]);
        commands.add_command("baz", &["Baz"]);
        commands.add_command("abiba", &["Abiba"]);

        let commands = Rc::new(commands);
        let action_bar = ActionBar::new(commands.clone());

        Ok(Self {
            settings: Settings::default(),
            project_dirs,
            commands,
            commands_map,
            action_bar,
            state: AppState {
                exit: false,
                action_bar: ActionBarState::new(),
                terminal: None,
            },
        })
    }

    fn post_init(&mut self) -> io::Result<()> {
        let path = self.project_dirs.config_dir().join("settings.json");
        let result = try_forward(|| {
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(err) => {
                    if err.kind() == io::ErrorKind::NotFound { return Ok(()) }
                    return Err(err.into())
                },
            };

            self.settings = serde_json::from_reader(BufReader::new(file))?;
            Ok(())
        }, || format!("Failed to read settings from '{}'", path.display()).to_string());

        if let Err(err) = result {
            MessageBox::new(self, "Error", &err.to_string()).run()?;
        }

        Ok(())
    }

    pub fn run(&mut self, terminal: DefaultTerminal) -> io::Result<()> {
        let terminal = Rc::new(RefCell::new(terminal));
        self.state.terminal = Some(terminal.clone());

        self.post_init()?;

        while !self.state.exit {
            terminal.borrow_mut().draw(|frame| self.render_to_frame(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn handle_events(&mut self) -> io::Result<()> {
        let ev = event::read()?;

        if self.state.action_bar.is_active() {
            match self.state.action_bar.handle_event(ev, &self.action_bar) {
                Response::Command(cmd) => {
                    if let Some(cmd) = self.commands_map.get(cmd) {
                        cmd(self);
                    }
                },
                _ => {},
            }
            return Ok(())
        }

        match ev {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.handle_key_press(key)
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_key_press(&mut self, key: event::KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.state.exit = true,
            KeyCode::Char(':') => self.state.action_bar.activate(ActionBarMode::Command, &self.action_bar),
            KeyCode::Char('/') => self.state.action_bar.activate(ActionBarMode::Search, &self.action_bar),
            _ => {}
        }
    }

    fn cmd_quit(&mut self) {
        self.state.exit = true;
    }

    fn cmd_account_add(&mut self) {
        todo!()
    }
}

impl TopWidget for App {
    fn terminal(&self) -> Rc<RefCell<DefaultTerminal>> {
        self.state.terminal.as_ref().unwrap().clone()
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let vertical = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
        let block = Block::default()
            .title("Pull Requests")
            .borders(Borders::TOP)
            .border_type(BorderType::Thick)
            .yellow();
        Paragraph::new("No accounts configured. Press ':' and select \"Add Account\"")
            .black()
            .on_white()
            .block(block)
            .render(vertical[0], buf);
        self.action_bar.render(vertical[1], buf, &mut self.state.action_bar);
    }
}

fn main() -> io::Result<()> {
    let mut app = App::init()?;

    let mut terminal = ratatui::init();
    terminal.clear()?;
    let app_result = app.run(terminal);
    ratatui::restore();

    app_result
}
