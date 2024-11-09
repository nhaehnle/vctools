use std::cell::RefCell;

use vctuik::{
    checkbox,
    event::{self, KeyCode},
    prelude::*,
};

fn main() -> Result<()> {
    let mut terminal = vctuik::init()?;

    let mut running = true;

    let mut foo = false;
    let mut bar = false;

    while running {
        let running = RefCell::new(&mut running);
        let callbacks = terminal.draw(|builder| {
            checkbox::add(builder, "Foo", &mut foo);
            checkbox::add(builder, "Bar", &mut bar);
            checkbox::add(builder, "Running", &running);
            event::on_key_press(builder, KeyCode::Char('q'), |_| {
                **running.borrow_mut() = false;
            });
        })?;
        terminal.wait_events(callbacks)?;
    }

    Ok(())
}
