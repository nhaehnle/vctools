
pub mod checkbox;
pub mod event;
pub mod label;
pub mod layout;
pub mod panes;
pub mod prelude;
pub mod state;
pub mod signals;
pub mod terminal;
pub mod theme;

pub fn init() -> prelude::Result<terminal::Terminal> {
    terminal::Terminal::init()
}
