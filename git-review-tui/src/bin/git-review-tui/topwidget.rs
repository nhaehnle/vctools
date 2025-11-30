use std::{cell::RefCell, rc::Rc};

use ratatui::{prelude::*, DefaultTerminal};

use vctuik::theme::Theme;

pub trait TopWidget {
    fn terminal(&self) -> Rc<RefCell<DefaultTerminal>>;
    fn theme(&self) -> &Theme;
    fn render(&mut self, area: Rect, buf: &mut Buffer);

    fn render_to_frame(&mut self, frame: &mut Frame) {
        struct WrapperWidget<'slf, W: ?Sized>(&'slf mut W);
        impl<'slf, W: TopWidget + ?Sized> Widget for WrapperWidget<'slf, W> {
            fn render(self, area: Rect, buf: &mut Buffer) {
                self.0.render(area, buf);
            }
        }
        frame.render_widget(WrapperWidget(self), frame.area());
    }
}

type PhantomUnsync = std::marker::PhantomData<std::cell::Cell<()>>;
type PhantomUnsend = std::marker::PhantomData<std::sync::MutexGuard<'static, ()>>;

pub struct StateMarker {}

pub struct StateRef<'t, T> {
    state: std::cell::RefCell<&'t mut T>,
    _unsend: PhantomUnsend,
    _unsync: PhantomUnsync,
}
impl<'t, T> StateRef<'t, T> {
    pub fn new(state: &'t mut T) -> Self {
        Self {
            state: std::cell::RefCell::new(state),
            _unsend: PhantomUnsend::default(),
            _unsync: PhantomUnsync::default(),
        }
    }
}

struct CheckBox {}
impl CheckBox {
    fn new(state: &mut bool) -> Self {
        CheckBox {}
    }

    fn render(&self) {}
}

struct Tui<'state> {
    _phantom: std::marker::PhantomData<&'state mut ()>,
}
impl<'state> Tui<'state> {
    fn new() -> Self {
        Tui {
            _phantom: std::marker::PhantomData,
        }
    }
}

struct TuiState<T> {
    data: RefCell<T>,
}
impl<T> TuiState<T> {
    pub fn new(data: T) -> Self {
        TuiState {
            data: RefCell::new(data),
        }
    }

    pub fn borrow(&self) -> std::cell::Ref<T> {
        self.data.borrow()
    }

    pub fn borrow_mut(&self, tui: &mut Tui) -> std::cell::RefMut<T> {
        self.data.borrow_mut()
    }
}

fn add_checkbox<'a>(tui: Tui<'a>, state: &'a TuiState<bool>) -> Tui<'a> {
    tui
}

fn run() {
    let mut blah = TuiState::new(false);

    let mut tui = Tui::new();

    loop {
        tui = add_checkbox(tui, &blah);
    }
}
