// SPDX-License-Identifier: GPL-3.0-or-later

pub mod check_box;
pub mod command;
pub mod event;
#[cfg(feature = "input-widget")]
pub mod input;
pub mod label;
pub mod layout;
pub mod pager;
pub mod prelude;
pub mod section;
pub mod state;
pub mod signals;
pub mod stringtools;
pub mod table;
pub mod terminal;
pub mod theme;

pub fn init() -> prelude::Result<terminal::Terminal> {
    terminal::Terminal::init()
}
