// SPDX-License-Identifier: GPL-3.0-or-later

use std::{any::Any, time::Instant};

use ratatui::{
    crossterm::{
        event::{DisableMouseCapture, EnableMouseCapture},
        execute,
    },
    layout::Position,
    widgets::Clear,
    DefaultTerminal,
};

use crate::{
    event::{self, Event, EventExt},
    layout::{self, Constraint1D},
    prelude::*,
    signals::{self, Dispatch, MergeWakeupWait, Receiver},
    state::{BuildStore, Builder, Store},
    theme::Theme,
};

struct Events {
    recv: Receiver<Result<Event>>,
    injected: Vec<Box<dyn Any + Send + Sync>>,
    wakeup_waits: Vec<MergeWakeupWait>,
}
impl Events {
    fn new() -> Self {
        let (signal, recv) = signals::make_channel();

        std::thread::spawn(move || {
            if let Err(err) = try_forward(
                || -> Result<()> {
                    loop {
                        signal.signal(Ok(event::read()?));
                    }
                },
                || "",
            ) {
                signal.signal(Err(err));
            }
        });

        Self {
            recv,
            injected: Vec::new(),
            wakeup_waits: Vec::new(),
        }
    }

    fn get(&mut self, wait: bool) -> Result<Option<EventExt>> {
        if !self.injected.is_empty() {
            return Ok(Some(EventExt::Custom(
                self.injected.drain(0..1).next().unwrap(),
            )));
        }

        let mut the_event = None;
        let mut the_err = None;
        let mut dispatch = Dispatch::new();
        dispatch.add(self.recv.dispatch_one(|event| match event {
            Ok(event) => {
                the_event = Some(EventExt::Event(event));
            }
            Err(err) => {
                the_err = Some(err);
            }
        }));
        for wait in &mut self.wakeup_waits {
            dispatch.add(wait.dispatch());
        }
        dispatch.poll(wait);

        if let Some(err) = the_err.take() {
            Err(err)?
        }

        Ok(the_event)
    }
}

pub struct Terminal {
    terminal: DefaultTerminal,
    store: Store,
    events: Events,
    theme: Theme,
    need_refresh: bool,
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

        Ok(Terminal {
            terminal,
            store: Store::default(),
            events: Events::new(),
            theme: Theme::default(),
            need_refresh: true,
        })
    }

    fn restore() {
        let mut stdout = std::io::stdout();
        if let Err(err) = execute!(stdout, DisableMouseCapture) {
            eprintln!("Failed to disable mouse capture: {err}");
        }
    }

    /// Add a waiter part of a merge wakeup pair.
    ///
    /// The terminal will refresh when the merge wakeup is signaled.
    pub fn add_merge_wakeup(&mut self, wakeup_wait: MergeWakeupWait) {
        self.events.wakeup_waits.push(wakeup_wait);
    }

    /// Run a default event loop until f returns false.
    pub fn run<F>(&mut self, mut f: F) -> Result<()>
    where
        F: FnMut(&mut Builder) -> Result<bool>,
    {
        let mut the_result = Ok(());
        let mut the_event: Option<EventExt> = None;
        let mut start_frame = Instant::now();
        let mut running = true;

        loop {
            self.terminal.draw(|frame| {
                the_result = || -> Result<()> {
                    loop {
                        // Process the UI once.
                        let area = frame.area();
                        let mut build_store = BuildStore::new(
                            &mut self.store,
                            &self.theme,
                            frame,
                            the_event.take(),
                            start_frame,
                        );

                        {
                            let mut layout = layout::LayoutEngine::new();
                            let mut builder = Builder::new(&mut build_store, &mut layout, area);
                            if !f(&mut builder)? {
                                running = false;
                                return Ok(());
                            }

                            if layout
                                .finish(
                                    Constraint1D::new_fixed(area.height),
                                    &mut build_store.current_layout_mut(),
                                )
                                .0
                            {
                                build_store.need_refresh = true;
                            }
                        }

                        build_store.end_frame();
                        self.events.injected.append(&mut build_store.injected);
                        self.need_refresh = build_store.need_refresh;

                        // If the UI hasn't settled, just re-process it immediately
                        // without an event (since the settling could affect how
                        // events are routed).
                        //
                        // If the UI has settled and there is another event, process
                        // it immediately.
                        //
                        // Finally, if the UI has settled and there are no more events
                        // to process, break out of the loop and actually send out
                        // the rendered frame.
                        if !self.need_refresh {
                            the_event = self.events.get(false)?;
                            if the_event.is_none() {
                                return Ok(());
                            }
                        }

                        // TODO: reset the cursor position properly instead of this
                        // out-of-bounds hack -- requires Ratatui change
                        frame.set_cursor_position(Position {
                            x: area.width,
                            y: area.height,
                        });
                        frame.render_widget(Clear, area);
                    }
                }();
            })?;

            if !running || the_result.is_err() {
                break;
            }

            the_event = self.events.get(true)?;
            start_frame = Instant::now();
        }

        the_result
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        Terminal::restore();
        ratatui::restore();
    }
}
