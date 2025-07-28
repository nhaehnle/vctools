
pub mod check_box;
pub mod event;
#[cfg(feature = "input-widget")]
pub mod input;
pub mod label;
pub mod layout;
pub mod prelude;
pub mod section;
pub mod state;
pub mod signals;
pub mod terminal;
pub mod theme;
#[cfg(feature = "tree-widget")]
pub mod tree;

pub fn init() -> prelude::Result<terminal::Terminal> {
    terminal::Terminal::init()
}
