
use vctuik_unsafe_internals::state::*;

fn main() {
    let mut store = Store::new();

    let a: &mut i32;

    {
        let mut builder = Builder::new(&mut store);
        a = builder.get_or_insert_default("a", None);
    }

    *a = 1;

    {
        Builder::new(&mut store);
    }

    *a;
}
