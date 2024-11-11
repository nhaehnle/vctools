use std::cell::RefCell;

use vctuik::{
    checkbox,
    panes::{Pane, Panes},
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
            let mut panes = Panes::new();
            panes.add(Pane::new("Foo"), |builder| {
                checkbox::add(builder, "Foo", &mut foo);
            });
            panes.add(Pane::new("Bar"), |builder| {
                checkbox::add(builder, "Bar", &mut bar);
            });
            panes.add(Pane::new("Running"), |builder| {
                checkbox::add(builder, "Running", &running);
            });
            panes.build(builder, "panes", builder.viewport().height);
            event::on_key_press(builder, KeyCode::Char('q'), |_| {
                **running.borrow_mut() = false;
            });
        })?;
        terminal.wait_events(callbacks)?;
    }

    Ok(())
}
