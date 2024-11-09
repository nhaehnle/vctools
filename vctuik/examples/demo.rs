
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
        let callbacks = terminal.draw(|builder| {
            checkbox::add(builder, "Foo", &mut foo);
            checkbox::add(builder, "Bar", &mut bar);
            //checkbox::add(context, "Running", &mut running);
            event::on_key_press(builder, KeyCode::Char('q'), |_| running = false);
        })?;
        terminal.wait_events(callbacks)?;
    }

    Ok(())
}
