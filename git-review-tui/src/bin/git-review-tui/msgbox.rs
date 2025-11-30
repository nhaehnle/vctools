use std::{cell::RefCell, io, rc::Rc};

use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    prelude::*,
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    DefaultTerminal,
};

use vctuik::theme::{Theme, Themed};

use crate::topwidget::TopWidget;

pub struct MessageBox<'slf> {
    title: &'slf str,
    message: &'slf str,
    parent: &'slf mut dyn TopWidget,
}
impl<'slf> MessageBox<'slf> {
    pub fn new(parent: &'slf mut dyn TopWidget, title: &'slf str, message: &'slf str) -> Self {
        Self {
            title,
            message,
            parent,
        }
    }

    pub fn run(mut self) -> io::Result<()> {
        loop {
            self.terminal()
                .borrow_mut()
                .draw(|frame| self.render_to_frame(frame))?;

            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if key.code == KeyCode::Esc || key.code == KeyCode::Enter {
                        break;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}
impl<'slf> TopWidget for MessageBox<'slf> {
    fn terminal(&self) -> Rc<RefCell<DefaultTerminal>> {
        self.parent.terminal()
    }

    fn theme(&self) -> &Theme {
        self.parent.theme()
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.parent.render(area, buf);

        const TW: u16 = 30;
        const TH: u16 = 15;
        let width = if area.width < TW {
            area.width
        } else {
            TW + (area.width - TW) / 3
        };
        let height = if area.height < TH {
            area.height
        } else {
            TH + (area.height - TH) / 3
        };

        let msg_area = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );

        Clear.render(msg_area, buf);

        Block::default()
            .title_alignment(Alignment::Center)
            .title(self.title)
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .style(self.theme().modal_background)
            .border_style(self.theme().modal_frame)
            .render(msg_area, buf);

        let inner_area = msg_area.inner(Margin::new(2, 2));
        Paragraph::new(self.message)
            .wrap(Wrap { trim: true })
            .style(self.theme().modal_text.normal)
            .render(inner_area, buf);
    }
}
