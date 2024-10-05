use std::{
    cell::RefCell,
    rc::Rc,
};

use ratatui::{prelude::*, DefaultTerminal};

pub trait TopWidget {
    fn terminal(&self) -> Rc<RefCell<DefaultTerminal>>;
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
