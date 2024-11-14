///! This crate contains internals of vctuik that make use of `unsafe`.
///!
///! The APIs exported by this crate are safe to use.
///!
///! No use of the `unsafe` keyword is allowed outside of this crate.

pub mod state;

#[cfg(test)]
mod test {
    #[test]
    fn ui() {
        let t = trybuild::TestCases::new();
        t.compile_fail("tests/ui/lifetime_check.rs");
    }
}
