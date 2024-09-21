use std::io;

use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::Stylize,
    text::Line,
    widgets::{
        Block, Paragraph, Widget
    },
    DefaultTerminal
};

use directories::ProjectDirs;

mod action;

use action::{ActionBar, ActionBarMode};

#[derive(Debug)]
struct App {
    project_dirs: ProjectDirs,
    exit: bool,
    action_bar: ActionBar,
}

impl App {
    pub fn init() -> io::Result<Self> {
        let project_dirs = ProjectDirs::from("", "", "git-review-tui").unwrap();

        std::fs::create_dir_all(&project_dirs.config_dir())?;
        std::fs::create_dir_all(&project_dirs.cache_dir())?;

        Ok(Self {
            project_dirs,
            exit: false,
            action_bar: ActionBar::new(),
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
        Paragraph::new("Hello Ratatui! (press 'q' to quit)")
            .white()
            .on_blue()
            .block(Block::bordered())
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
