use std::cell::RefCell;

use vctuik::{
    checkbox::add_check_box,
    label::add_label,
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
            panes.add(Pane::new("Settings"), |builder| {
                add_check_box(builder, "Foo", &mut foo);
                add_check_box(builder, "Bar", &mut bar);
            });
            panes.add(Pane::new("Commentary"), |builder| {
                add_label(builder, "Cheddar");
                add_label(builder, "Provolone");
                add_label(builder, "Swiss");
            });
            panes.add(Pane::new("Running"), |builder| {
                add_check_box(builder, "Running", &running);
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
