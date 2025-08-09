use ratatui::{
    crossterm::{
        event::{DisableMouseCapture, EnableMouseCapture},
        execute,
    }, DefaultTerminal
};

use crate::{
    event::{self, Event},
    layout::{self, Constraint1D},
    prelude::*,
    signals::{self, Dispatch, Receiver},
    state::{BuildStore, Builder, Store},
    theme::Theme,
};

pub struct Terminal {
    terminal: DefaultTerminal,
    store: Store,
    event_recv: Receiver<Result<Event>>,
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
            need_refresh: true,
        })
    }

    fn restore() {
        let mut stdout = std::io::stdout();
        if let Err(err) = execute!(stdout, DisableMouseCapture) {
            eprintln!("Failed to disable mouse capture: {err}");
        }
    }

    /// Run one frame.
    ///
    /// Returns true if the frame should be run again immediately as some refresh is needed.
    pub fn run_frame<F>(&mut self, f: F) -> Result<bool>
    where
        F: FnOnce(&mut Builder),
    {
        let mut the_event = None;

        if !self.need_refresh {
            let mut the_err = None;
            let mut dispatch = Dispatch::new();
            dispatch.add(self.event_recv.dispatch_one(|event| {
                match event {
                    Ok(event) => {
                        the_event = Some(event);
                    }
                    Err(err) => {
                        the_err = Some(err);
                    }
                }
            }));
            dispatch.poll(true);

            if let Some(err) = the_err.take() {
                Err(err)?
            }

            assert!(the_event.is_some());
        }

        self.terminal.draw(|frame| {
            let area = frame.area();
            let mut build_store = BuildStore::new(&mut self.store, &self.theme, frame, the_event);

            {
                let mut layout = layout::LayoutEngine::new();
                let mut builder = Builder::new(&mut build_store, &mut layout, area);
                f(&mut builder);
                if layout.finish(Constraint1D::new_fixed(area.height), &mut build_store.current_layout_mut()).0 {
                    self.need_refresh = true;
                }
            }

            build_store.end_frame();
            self.need_refresh = build_store.need_refresh;
        })?;

        Ok(self.need_refresh)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        Terminal::restore();
        ratatui::restore();
    }
}
