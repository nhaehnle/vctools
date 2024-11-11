use std::sync::LazyLock;

use ratatui::{
    prelude::*,
    style::{Style, Styled},
};

use crate::state::Builder;

#[derive(Debug, Clone)]
pub struct Text {
    pub normal: Style,
    pub highlight: Style,
    pub inactive: Style,
    pub selected: Style,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    None,
    Pane,
    Modal,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub text: Text,
    pub pane_background: Style,
    pub pane_frame_normal: Style,
    pub pane_frame_focus: Style,
    pub pane_text: Text,
    pub modal_background: Style,
    pub modal_frame: Style,
    pub modal_text: Text,
}
impl Theme {
    pub fn text(&self, context: Context) -> &Text {
        match context {
            Context::None => &self.text,
            Context::Pane => &self.pane_text,
            Context::Modal => &self.modal_text,
        }
    }
}
impl Default for Theme {
    fn default() -> Self {
        SOLARIZED_LIGHT.clone()
    }
}

fn make_solarized(dark: bool) -> Theme {
    // The original Solarized color theme is:
    //
    // Copyright (c) 2011 Ethan Schoonover
    // 
    // Permission is hereby granted, free of charge, to any person obtaining a copy
    // of this software and associated documentation files (the "Software"), to deal
    // in the Software without restriction, including without limitation the rights
    // to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    // copies of the Software, and to permit persons to whom the Software is
    // furnished to do so, subject to the following conditions:
    // 
    // The above copyright notice and this permission notice shall be included in
    // all copies or substantial portions of the Software.
    // 
    // THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    // IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    // FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    // AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    // LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    // OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
    // THE SOFTWARE.
    let mut base03 =    Color::Rgb(0x00, 0x2b, 0x36);
    let mut base02 =    Color::Rgb(0x07, 0x36, 0x42);
    let mut base01 =    Color::Rgb(0x58, 0x6e, 0x75);
    let mut base00 =    Color::Rgb(0x65, 0x7b, 0x83);
    let mut base0 =     Color::Rgb(0x83, 0x94, 0x96);
    let mut base1 =     Color::Rgb(0x93, 0xa1, 0xa1);
    let mut base2 =     Color::Rgb(0xee, 0xe8, 0xd5);
    let mut base3 =     Color::Rgb(0xfd, 0xf6, 0xe3);
    let yellow =        Color::Rgb(0xb5, 0x89, 0x00);
    let _orange =       Color::Rgb(0xcb, 0x4b, 0x16);
    let _red =          Color::Rgb(0xdc, 0x32, 0x2f);
    let _magenta =      Color::Rgb(0xd3, 0x36, 0x82);
    let _violet =       Color::Rgb(0x6c, 0x71, 0xc4);
    let _blue =         Color::Rgb(0x26, 0x8b, 0xd2);
    let _cyan =         Color::Rgb(0x2a, 0xa1, 0x98);
    let _green =        Color::Rgb(0x85, 0x99, 0x00);

    if dark {
        std::mem::swap(&mut base0, &mut base00);
        std::mem::swap(&mut base1, &mut base01);
        std::mem::swap(&mut base2, &mut base02);
        std::mem::swap(&mut base3, &mut base03);
    }

    Theme {
        text: Text {
            normal: Style::default().fg(base00),
            highlight: Style::default().fg(yellow),
            inactive: Style::default().fg(base1),
            selected: Style::default().bg(base02).fg(base1),
        },
        pane_background: Style::default().bg(base3),
        pane_frame_normal: Style::default().fg(base00).bold(),
        pane_frame_focus: Style::default().fg(yellow).bold(),
        pane_text: Text {
            normal: Style::default().fg(base00),
            highlight: Style::default().fg(yellow),
            inactive: Style::default().fg(base1),
            selected: Style::default().bg(base02).fg(base1),
        },
        modal_background: Style::default().bg(base2),
        modal_frame: Style::default().fg(yellow).bold(),
        modal_text: Text {
            normal: Style::default().fg(base00),
            highlight: Style::default().fg(yellow),
            inactive: Style::default().fg(base1),
            selected: Style::default().bg(base02).fg(base1),
        },
    }
}

pub static SOLARIZED_LIGHT: LazyLock<Theme> = LazyLock::new(|| {
    make_solarized(false)
});
pub static SOLARIZED_DARK: LazyLock<Theme> = LazyLock::new(|| {
    make_solarized(true)
});

pub trait Themed {
    type Item;

    fn theme_text(self, theme: &Builder) -> Self::Item;
    fn theme_highlight(self, theme: &Builder) -> Self::Item;
    fn theme_inactive(self, theme: &Builder) -> Self::Item;
    fn theme_selected(self, theme: &Builder) -> Self::Item;
}
impl<I: Styled<Item = I>> Themed for I {
    type Item = I;

    fn theme_text(self, builder: &Builder) -> Self::Item {
        self.set_style(builder.theme().text(builder.context()).normal)
    }

    fn theme_highlight(self, builder: &Builder) -> Self::Item {
        self.set_style(builder.theme().text(builder.context()).highlight)
    }

    fn theme_inactive(self, builder: &Builder) -> Self::Item {
        self.set_style(builder.theme().text(builder.context()).inactive)
    }

    fn theme_selected(self, builder: &Builder) -> Self::Item {
        self.set_style(builder.theme().text(builder.context()).selected)
    }
}
