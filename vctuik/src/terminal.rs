use std::cell::Cell;

use ratatui::DefaultTerminal;

use crate::{
    event::{self, Event},
    prelude::*,
    signals::{self, Dispatch, Receiver},
    state::{BuildStore, Builder, EventHandlers, GlobalEventHandler, Handled, StateStore},
    theme::Theme,
};

pub struct Terminal {
    terminal: DefaultTerminal,
    state_store: StateStore,
    event_recv: Receiver<Result<Event>>,
    theme: Theme,
}
impl Terminal {
    pub(crate) fn init() -> Result<Terminal> {
        let mut terminal = ratatui::try_init()?;
        terminal.clear()?;

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
            state_store: StateStore::default(),
            event_recv,
            theme: Theme::default(),
        })
    }

    pub fn draw<'slf, 'render, 'handler, F>(&'slf mut self, f: F) -> Result<EventHandlers<'handler>>
    where
        F: FnOnce(&mut Builder<'_, 'render, 'handler>),
        'slf: 'render,
    {
        let mut result = None;

        self.terminal.draw(|frame| {
            let mut build_store = BuildStore::new(std::mem::take(&mut self.state_store), &self.theme);
            f(&mut Builder::new(&mut build_store, frame.area()));

            self.state_store = build_store.state;

            for renderable in build_store.render {
                renderable.render(frame);
            }

            result = Some(build_store.handlers);
        })?;

        Ok(result.unwrap())
    }

    pub fn wait_events<'a>(&'a mut self, mut handlers: EventHandlers<'a>) -> Result<()> {
        let the_err = Cell::new(None);

        handlers.push(Box::new(GlobalEventHandler::new(&mut self.state_store.current)));

        // TODO: Need to bail out of this loop if there is a state change that
        //       changes how events are routed (e.g. TAB press).

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
        match event {
            Event::Key(key_event) => {
                for handler in handlers {
                    let handled = handler.handle_key_event(key_event);
                    if handled != Handled::No {
                        return handled;
                    }
                }

                Handled::No
            },
            _ => { Handled::No },
        }
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        ratatui::restore();
    }
}
