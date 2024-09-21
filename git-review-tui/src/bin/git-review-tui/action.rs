
use ratatui::{
    buffer::Buffer,
    crossterm::event::{Event, KeyCode, KeyEventKind},
    layout::Rect,
    style::Stylize,
    widgets::{Paragraph, Widget},
};

use tui_input::{backend::crossterm::EventHandler, Input};

#[derive(Debug)]
pub struct Commands {
}

impl Commands {
    fn new() -> Self {
        Self {
        }
    }
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
pub struct ActionBar(ActionBarState);

impl ActionBar {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        matches!(self.0, ActionBarState::Active { .. })
    }

    pub fn activate(&mut self, mode: ActionBarMode) {
        self.0 = ActionBarState::Active {
            input: Input::new((match mode {
                ActionBarMode::Command => ":",
                ActionBarMode::Search => "/",
            }).into()),
        };
    }

    pub fn handle_event(&mut self, ev: Event) -> Response {
        let ActionBarState::Active { input } = &mut self.0 else { return Response::Cancel };

        match ev {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match key.code {
                    KeyCode::Enter => {
                        self.0 = ActionBarState::Idle;
                        Response::Cancel
                    }
                    KeyCode::Esc => {
                        self.0 = ActionBarState::Idle;
                        Response::Cancel
                    }
                    _ => {
                        input.handle_event(&ev);
                        if input.value().is_empty() {
                            self.0 = ActionBarState::Idle;
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

impl Default for ActionBar {
    fn default() -> Self {
        Self(ActionBarState::Idle)
    }
}

impl Widget for &ActionBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match &self.0 {
            ActionBarState::Idle => {
                Paragraph::new("Press ':' to enter command mode, '/' to enter search mode")
                    .blue()
                    .on_white()
                    .render(area, buf);
            }
            ActionBarState::Active { input, .. } => {
                let scroll = input.visual_scroll(area.width as usize);
                Paragraph::new(input.value())
                    .blue()
                    .on_white()
                    .scroll((0, scroll as u16))
                    .render(area, buf);
            }
        }
    }
}
