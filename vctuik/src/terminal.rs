use std::cell::Cell;

use ratatui::{
    crossterm::{
        event::{DisableMouseCapture, EnableMouseCapture},
        execute,
    }, DefaultTerminal
};

use crate::{
    event::{self, Event},
    prelude::*,
    signals::{self, Dispatch, Receiver},
    state::{BuildStore, Builder, EventHandlers, Handled, Store},
    theme::Theme,
};

pub struct Terminal {
    terminal: DefaultTerminal,
    store: Store,
    event_recv: Receiver<Result<Event>>,
    theme: Theme,
}
impl Terminal {
    pub(crate) fn init() -> Result<Terminal> {
        let mut terminal = ratatui::try_init()?;
        terminal.clear()?;

        let mut stdout = std::io::stdout();
        execute!(stdout, EnableMouseCapture)?;

        let old_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            Terminal::restore();
            old_hook(info);
        }));

        let (event_signal, event_recv) = signals::make_channel();

        std::thread::spawn(
            move || {
                if let Err(err) = try_forward(|| -> Result<()> {
                    loop {
                        event_signal.signal(Ok(event::read()?));
                    }
                }, || "") {
                    event_signal.signal(Err(err));
                }
            });

        Ok(Terminal {
            terminal,
            store: Store::default(),
            event_recv,
            theme: Theme::default(),
        })
    }

    fn restore() {
        let mut stdout = std::io::stdout();
        if let Err(err) = execute!(stdout, DisableMouseCapture) {
            eprintln!("Failed to disable mouse capture: {err}");
        }
    }

    pub fn run_frame<'slf, 'render, 'handler, F>(&'slf mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut Builder<'_, 'render, 'handler>),
        'slf: 'render + 'handler,
    {
        let mut result = None;

        self.terminal.draw(|frame| {
            let mut build_store = BuildStore::new(&mut self.store, &self.theme);
            f(&mut Builder::new(&mut build_store, frame.area()));

            let (render, handlers) = build_store.finish();

            for renderable in render {
                renderable.render(frame);
            }

            result = Some(handlers);
        })?;

        let mut handlers = result.unwrap();

        // TODO: Need to bail out of this loop if there is a state change that
        //       changes how events are routed (e.g. TAB press).

        let the_err = Cell::new(None);
        let mut dispatch = Dispatch::new();
        let mut event_recv = self.event_recv.dispatch(|event| {
            match event {
                Ok(event) => {
                    Terminal::handle_event(event, &mut handlers);
                }
                Err(err) => {
                    the_err.set(Some(err));
                }
            }
        });
        dispatch.add(&mut event_recv);
        dispatch.wait_then_poll();

        match the_err.take() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn handle_event(event: Event, handlers: &mut EventHandlers<'_>) -> Handled {
        for handler in handlers {
            let handled = handler(&event);
            if handled != Handled::No {
                return handled;
            }
        }
        Handled::No
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        Terminal::restore();
        ratatui::restore();
    }
}
