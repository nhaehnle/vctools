///! This crate contains internals of vctuik that make use of `unsafe`.
///!
///! The APIs exported by this crate are safe to use.
///!
///! No use of the `unsafe` keyword is allowed outside of this crate.

pub mod state;

use std::ptr;

#[cfg(test)]
mod test {
    #[test]
    fn ui() {
        let t = trybuild::TestCases::new();
        t.compile_fail("tests/ui/lifetime_check.rs");
    }
}

pub fn update_mut<T>(x: &mut T, f: impl FnOnce(T) -> T) {
    // Safety: Since we hold an exclusive reference to x, we can safely read and write its value.
    //
    // The one concern is that f may panic, leaving x in an inconsistent state.
    let mut x_value = unsafe { ptr::read(x) };
    x_value = f(x_value);
    unsafe { ptr::write(x, x_value); };
}
