
pub mod check_box;
pub mod event;
pub mod label;
pub mod layout;
pub mod panes;
pub mod prelude;
pub mod state;
pub mod signals;
pub mod terminal;
pub mod theme;
#[cfg(feature = "tree-widget")]
pub mod tree;

pub fn init() -> prelude::Result<terminal::Terminal> {
    terminal::Terminal::init()
}
