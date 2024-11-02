
use ratatui::{
    prelude::*,
    style::{Style, Styled},
};

#[derive(Debug)]
pub struct Theme {
    pub pane_active: Style,
    pub pane_inactive: Style,
    pub content: Style,
    pub highlight: Style,
    pub selection: Style,
    pub modal_pane: Style,
    pub modal_content: Style,
    pub modal_highlight: Style,
    pub modal_selection: Style,
    pub status_bar: Style,
}

impl Default for Theme {
    fn default() -> Self {
        let light_blue = Color::Rgb(0, 0, 255);
        let dark_blue = Color::Rgb(0, 0, 160);
        let yellow = Color::Rgb(255, 255, 0);
        let light_yellow = Color::Rgb(250, 250, 210);
        let white = Color::Rgb(255, 255, 255);
        let gray = Color::Rgb(128, 128, 128);
        let light_gray = Color::Rgb(160, 160, 160);
        let dark_gray = Color::Rgb(88, 88, 88);

        Theme {
            pane_active: Style::default().bg(light_blue).fg(yellow),
            pane_inactive: Style::default().bg(dark_blue).fg(gray),
            content: Style::default().fg(white),
            highlight: Style::default().fg(yellow),
            selection: Style::default().bg(light_gray).fg(light_blue),
            modal_pane: Style::default().bg(light_yellow).fg(dark_blue),
            modal_content: Style::default().fg(dark_gray),
            modal_highlight: Style::default().fg(dark_blue),
            modal_selection: Style::default().bg(light_blue).fg(white),
            status_bar: Style::default().bg(light_yellow).fg(dark_gray),
        }
    }
}

pub trait Themed {
    type Item;

    fn set_style_opt(self, style: Option<Style>) -> Self::Item;

    fn theme_pane_active(self, theme: &Theme) -> Self::Item;
    fn theme_pane_inactive(self, theme: &Theme) -> Self::Item;
    fn theme_content(self, theme: &Theme) -> Self::Item;
    fn theme_highlight(self, theme: &Theme) -> Self::Item;
    fn theme_selection(self, theme: &Theme) -> Self::Item;
    fn theme_modal_pane(self, theme: &Theme) -> Self::Item;
    fn theme_modal_content(self, theme: &Theme) -> Self::Item;
    fn theme_modal_highlight(self, theme: &Theme) -> Self::Item;
    fn theme_modal_selection(self, theme: &Theme) -> Self::Item;
    fn theme_status_bar(self, theme: &Theme) -> Self::Item;

    fn opt_theme_pane_active(self, theme: Option<&Theme>) -> Self::Item;
    fn opt_theme_pane_inactive(self, theme: Option<&Theme>) -> Self::Item;
    fn opt_theme_content(self, theme: Option<&Theme>) -> Self::Item;
    fn opt_theme_highlight(self, theme: Option<&Theme>) -> Self::Item;
    fn opt_theme_selection(self, theme: Option<&Theme>) -> Self::Item;
    fn opt_theme_modal_pane(self, theme: Option<&Theme>) -> Self::Item;
    fn opt_theme_modal_content(self, theme: Option<&Theme>) -> Self::Item;
    fn opt_theme_modal_highlight(self, theme: Option<&Theme>) -> Self::Item;
    fn opt_theme_modal_selection(self, theme: Option<&Theme>) -> Self::Item;
    fn opt_theme_status_bar(self, theme: Option<&Theme>) -> Self::Item;
}
impl<I: Styled<Item = I>> Themed for I {
    type Item = I;

    fn set_style_opt(self, style: Option<Style>) -> Self::Item {
        if let Some(style) = style {
            self.set_style(style)
        } else {
            self
        }
    }

    fn theme_pane_active(self, theme: &Theme) -> Self::Item {
        self.set_style(theme.pane_active)
    }

    fn theme_pane_inactive(self, theme: &Theme) -> Self::Item {
        self.set_style(theme.pane_inactive)
    }

    fn theme_content(self, theme: &Theme) -> Self::Item {
        self.set_style(theme.content)
    }

    fn theme_highlight(self, theme: &Theme) -> Self::Item {
        self.set_style(theme.highlight)
    }

    fn theme_selection(self, theme: &Theme) -> Self::Item {
        self.set_style(theme.selection)
    }

    fn theme_modal_pane(self, theme: &Theme) -> Self::Item {
        self.set_style(theme.modal_pane)
    }

    fn theme_modal_content(self, theme: &Theme) -> Self::Item {
        self.set_style(theme.modal_content)
    }

    fn theme_modal_highlight(self, theme: &Theme) -> Self::Item {
        self.set_style(theme.modal_highlight)
    }

    fn theme_modal_selection(self, theme: &Theme) -> Self::Item {
        self.set_style(theme.modal_selection)
    }

    fn theme_status_bar(self, theme: &Theme) -> Self::Item {
        self.set_style(theme.status_bar)
    }

    fn opt_theme_pane_active(self, theme: Option<&Theme>) -> Self::Item {
        if let Some(theme) = theme {
            self.set_style(theme.pane_active)
        } else {
            self
        }
    }

    fn opt_theme_pane_inactive(self, theme: Option<&Theme>) -> Self::Item {
        if let Some(theme) = theme {
            self.set_style(theme.pane_inactive)
        } else {
            self
        }
    }

    fn opt_theme_content(self, theme: Option<&Theme>) -> Self::Item {
        if let Some(theme) = theme {
            self.set_style(theme.content)
        } else {
            self
        }
    }

    fn opt_theme_highlight(self, theme: Option<&Theme>) -> Self::Item {
        if let Some(theme) = theme {
            self.set_style(theme.highlight)
        } else {
            self
        }
    }

    fn opt_theme_selection(self, theme: Option<&Theme>) -> Self::Item {
        if let Some(theme) = theme {
            self.set_style(theme.selection)
        } else {
            self
        }
    }

    fn opt_theme_modal_pane(self, theme: Option<&Theme>) -> Self::Item {
        if let Some(theme) = theme {
            self.set_style(theme.modal_pane)
        } else {
            self
        }
    }

    fn opt_theme_modal_content(self, theme: Option<&Theme>) -> Self::Item {
        if let Some(theme) = theme {
            self.set_style(theme.modal_content)
        } else {
            self
        }
    }

    fn opt_theme_modal_highlight(self, theme: Option<&Theme>) -> Self::Item {
        if let Some(theme) = theme {
            self.set_style(theme.modal_highlight)
        } else {
            self
        }
    }

    fn opt_theme_modal_selection(self, theme: Option<&Theme>) -> Self::Item {
        if let Some(theme) = theme {
            self.set_style(theme.modal_selection)
        } else {
            self
        }
    }

    fn opt_theme_status_bar(self, theme: Option<&Theme>) -> Self::Item {
        if let Some(theme) = theme {
            self.set_style(theme.status_bar)
        } else {
            self
        }
    }
}
