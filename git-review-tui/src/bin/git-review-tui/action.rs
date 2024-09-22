
use std::{borrow::Borrow, rc::Rc};

use ratatui::{
    buffer::Buffer,
    crossterm::event::{Event, KeyCode, KeyEventKind},
    layout::Rect,
    style::Stylize,
    widgets::{Paragraph, Widget},
};

use tui_input::{backend::crossterm::EventHandler, Input};

#[derive(Debug)]
struct Command {
    name: String,
    titles: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Commands {
    commands: Vec<Command>,
}

impl Commands {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_command<N, T>(&mut self, name: N, titles: &[T])
    where
        N: Into<String>,
        T: Copy + Into<String>,
    {
        self.commands.push(Command {
            name: name.into(),
            titles: titles.iter().map(|t| (*t).into()).collect(),
        });
    }

    // pub fn filter(&self, query: &str) -> Vec<&Command> {
        // self.commands
            // .iter()
            // .filter(|cmd| cmd.title.contains(query))
            // .collect()
    // }
}

#[derive(Debug, Clone, Copy)]
pub enum ActionBarMode {
    Command,
    Search,
}

pub enum Response {
    None,
    Cancel,
}

#[derive(Debug)]
enum ActionBarState {
    Idle,
    Active {
        input: Input,
    },
}

#[derive(Debug)]
pub struct ActionBar {
    state: ActionBarState,
    commands: Rc<Commands>,
}

impl ActionBar {
    pub fn new(commands: Rc<Commands>) -> Self {
        Self {
            state: ActionBarState::Idle,
            commands,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, ActionBarState::Active { .. })
    }

    pub fn activate(&mut self, mode: ActionBarMode) {
        self.state = ActionBarState::Active {
            input: Input::new((match mode {
                ActionBarMode::Command => ":",
                ActionBarMode::Search => "/",
            }).into()),
        };
    }

    pub fn handle_event(&mut self, ev: Event) -> Response {
        let ActionBarState::Active { input } = &mut self.state else { return Response::Cancel };

        match ev {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match key.code {
                    KeyCode::Enter => {
                        self.state = ActionBarState::Idle;
                        Response::Cancel
                    }
                    KeyCode::Esc => {
                        self.state = ActionBarState::Idle;
                        Response::Cancel
                    }
                    _ => {
                        input.handle_event(&ev);
                        if input.value().is_empty() {
                            self.state = ActionBarState::Idle;
                            Response::Cancel
                        } else {
                            Response::None
                        }
                    }
                }
            }
            _ => { Response::None }
        }
    }
}

impl Widget for &ActionBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match &self.state {
            ActionBarState::Idle => {
                Paragraph::new("Press ':' to enter command mode, '/' to enter search mode")
                    .blue()
                    .on_gray()
                    .render(area, buf);
            }
            ActionBarState::Active { input, .. } => {
                let scroll = input.visual_scroll(area.width as usize);
                Paragraph::new(input.value())
                    .blue()
                    .on_gray()
                    .scroll((0, scroll as u16))
                    .render(area, buf);
            }
        }
    }
}
