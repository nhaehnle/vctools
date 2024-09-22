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
        Block, Borders, BorderType, Paragraph, Widget
    },
    DefaultTerminal
};

use directories::ProjectDirs;

mod action;

use action::{ActionBar, ActionBarMode, Commands};

#[derive(Debug)]
struct App {
    project_dirs: ProjectDirs,
    exit: bool,
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
        let commands = Rc::new(commands);
        let action_bar = ActionBar::new(commands.clone());

        Ok(Self {
            project_dirs,
            exit: false,
            commands,
            action_bar,
        })
    }

    pub fn run(&mut self, mut terminal: DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| frame.render_widget(&*self, frame.area()))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn handle_events(&mut self) -> io::Result<()> {
        let ev =  event::read()?;

        if self.action_bar.is_active() {
            self.action_bar.handle_event(ev);
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
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Char(':') => self.action_bar.activate(ActionBarMode::Command),
            KeyCode::Char('/') => self.action_bar.activate(ActionBarMode::Search),
            _ => {}
        }
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
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
        self.action_bar.render(vertical[1], buf);
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
