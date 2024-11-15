use std::{any::Any, collections::{hash_map, HashMap}, hash::Hash};

pub struct Store<K> {
    previous: HashMap<K, Box<dyn Any>>,
    current: HashMap<K, Box<dyn Any>>,
}
impl<K> Store<K> {
    pub fn new() -> Self {
        Self::default()
    }
}
impl<K> Default for Store<K> {
    fn default() -> Self {
        Self {
            previous: HashMap::new(),
            current: HashMap::new(),
        }
    }
}

pub struct Builder<'frame, K> {
    store: &'frame mut Store<K>,
}
impl<'frame, K: Eq + Hash> Builder<'frame, K> {
    pub fn new(store: &'frame mut Store<K>) -> Self {
        std::mem::swap(&mut store.previous, &mut store.current);
        store.current.clear();

        Self { store }
    }

    pub fn entry<'entry, T: 'static>(&'entry mut self, new_key: K, old_key: Option<K>)
            -> Access<'frame, 'entry, K, T> {
        let entry = self.store.current.entry(new_key);
        match entry {
            hash_map::Entry::Occupied(_) =>
                panic!("Key inserted again in the same frame"),
            hash_map::Entry::Vacant(entry) => {
                let previous =
                    old_key
                        .and_then(|key| self.store.previous.remove(&key))
                        .filter(|value| value.is::<T>());
                match previous {
                    Some(value) => Access::Existing(Insert::new(entry).insert_box(value)),
                    None => Access::New(Insert::new(entry)),
                }
            }
        }
    }

    pub fn get_or_insert_with<T, F>(&mut self, new_key: K, old_key: Option<K>, f: F) -> &'frame mut T
    where
        T: 'static,
        F: FnOnce() -> T,
    {
        match self.entry(new_key, old_key) {
            Access::Existing(value) => value,
            Access::New(insert) => insert.insert(f())
        }
    }

    pub fn get_or_insert_default<T>(&mut self, new_key: K, old_key: Option<K>) -> &'frame mut T
    where
        T: Default + 'static,
    {
        self.get_or_insert_with(new_key, old_key, Default::default)
    }
}

pub enum Access<'frame, 'entry, K, T> {
    Existing(&'frame mut T),
    New(Insert<'frame, 'entry, K, T>),
}

pub struct Insert<'frame, 'entry, K, T> {
    entry: hash_map::VacantEntry<'entry, K, Box<dyn Any>>,
    _marker: std::marker::PhantomData<&'frame mut T>,
}
impl<'frame, 'entry, K, T: 'static> Insert<'frame, 'entry, K, T> {
    fn new(entry: hash_map::VacantEntry<'entry, K, Box<dyn Any>>) -> Self {
        Self {
            entry,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn insert(self, value: T) -> &'frame mut T {
        self.insert_box(Box::new(value))
    }

    fn insert_box(self, value: Box<dyn Any>) -> &'frame mut T {
        let the_box = self.entry.insert(value);
        let ptr = the_box.downcast_mut().unwrap() as *mut T;

        // SAFETY: The returned reference is valid for 'frame. The `Builder`
        // that created `self` has an exclusive reference to the underlying
        // `Store` for 'frame. This means that Builder::new cannot be called
        // again with the same store during 'frame.
        //
        // Furthermore, the panic in Builder::entry guarantees that the entry
        // cannot be accessed again through the same Builder.
        //
        // This means that:
        //  - the box contents cannot be freed during 'frame
        //  - nobody else can deref the box during 'frame
        //
        // Therefore, the returned reference is indeed valid and exclusive for
        // 'frame.
        unsafe { &mut *ptr }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn simple() {
        let mut store = Store::new();

        let mut a: &mut i32;
        let mut b: &mut i32;

        {
            let mut builder = Builder::new(&mut store);
            a = builder.get_or_insert_default("a", Some("a"));
            b = builder.get_or_insert_default("b", None);
        }

        assert_eq!(*a, 0);
        assert_eq!(*b, 0);

        *a = 1;
        *b = 2;

        {
            let mut builder = Builder::new(&mut store);
            a = builder.get_or_insert_default("a", None);
            b = builder.get_or_insert_default("b", Some("b"));
        }

        assert_eq!(*a, 0);
        assert_eq!(*b, 2);
    }
}
