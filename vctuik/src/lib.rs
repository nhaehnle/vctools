
pub mod checkbox;
pub mod event;
pub mod prelude;
pub mod state;
pub mod signals;
pub mod terminal;

pub fn init() -> prelude::Result<terminal::Terminal> {
    terminal::Terminal::init()
}
