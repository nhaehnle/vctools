use std::{
    io,
    rc::Rc,
};

use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::Stylize,
    text::Line,
    widgets::{
        Block, Borders, BorderType, Paragraph, StatefulWidget, Widget
    },
    DefaultTerminal
};

use directories::ProjectDirs;

mod action;

use action::{ActionBar, ActionBarMode, ActionBarState, Commands};

#[derive(Debug)]
struct AppState {
    exit: bool,
    action_bar: ActionBarState,
}

#[derive(Debug)]
struct App {
    project_dirs: ProjectDirs,
    commands: Rc<Commands>,
    action_bar: ActionBar,
}

impl App {
    pub fn init() -> io::Result<Self> {
        let project_dirs = ProjectDirs::from("", "", "git-review-tui").unwrap();

        std::fs::create_dir_all(&project_dirs.config_dir())?;
        std::fs::create_dir_all(&project_dirs.cache_dir())?;

        let mut commands = Commands::new();
        commands.add_command("quit", &["Quit", "Exit"]);

        commands.add_command("foo", &["Foo"]);
        commands.add_command("bar", &["Bar"]);
        commands.add_command("baz", &["Baz"]);
        commands.add_command("abiba", &["Abiba"]);

        let commands = Rc::new(commands);
        let action_bar = ActionBar::new(commands.clone());

        Ok(Self {
            project_dirs,
            commands,
            action_bar,
        })
    }

    pub fn run(&mut self, mut terminal: DefaultTerminal) -> io::Result<()> {
        let mut state = AppState {
            exit: false,
            action_bar: ActionBarState::new(),
        };
        while !state.exit {
            terminal.draw(|frame| frame.render_stateful_widget(&*self, frame.area(), &mut state))?;
            self.handle_events(&mut state)?;
        }
        Ok(())
    }

    fn handle_events(&mut self, state: &mut AppState) -> io::Result<()> {
        let ev =  event::read()?;

        if state.action_bar.is_active() {
            state.action_bar.handle_event(ev, &self.action_bar);
            return Ok(())
        }

        match ev {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.handle_key_press(state, key)
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_key_press(&mut self, state: &mut AppState, key: event::KeyEvent) {
        match key.code {
            KeyCode::Char('q') => state.exit = true,
            KeyCode::Char(':') => state.action_bar.activate(ActionBarMode::Command, &self.action_bar),
            KeyCode::Char('/') => state.action_bar.activate(ActionBarMode::Search, &self.action_bar),
            _ => {}
        }
    }
}

impl StatefulWidget for &App {
    type State = AppState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut AppState) {
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
        self.action_bar.render(vertical[1], buf, &mut state.action_bar);
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
