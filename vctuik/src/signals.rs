use std::sync::{Arc, Condvar, Mutex};

pub trait Dispatchable {
    fn poll(&mut self, wake: Waker) -> Option<Waker>;
}

struct Channel<Item> {
    items: Vec<Item>,
    waker: Option<Waker>,
}

pub struct Sender<Item> {
    channel: Arc<Mutex<Channel<Item>>>,
}
impl<Item> Sender<Item> {
    pub fn signal(&self, item: Item) {
        let mut channel = self.channel.lock().unwrap();
        channel.items.push(item);
        if let Some(waker) = channel.waker.take() {
            waker.wake();
        }
    }
}

pub struct Receiver<Item> {
    channel: Arc<Mutex<Channel<Item>>>,
}
impl<Item> Receiver<Item> {
    pub fn dispatch<'recv, F>(&'recv mut self, f: F) -> impl Dispatchable + 'recv
    where
        F: FnMut(Item) + 'recv
    {
        struct Impl<'recv, Item, F> {
            channel: &'recv Mutex<Channel<Item>>,
            f: F,
        }
        impl<'recv, Item, F: FnMut(Item) + 'recv> Dispatchable for Impl<'recv, Item, F> {
            fn poll(&mut self, wake: Waker) -> Option<Waker> {
                let (result, items) = {
                    let mut channel = self.channel.lock().unwrap();
                    if channel.items.is_empty() {
                        channel.waker = Some(wake);
                        (None, Vec::new())
                    } else {
                        (Some(wake), std::mem::replace(&mut channel.items, Vec::new()))
                    }
                };
                for item in items {
                    (self.f)(item);
                }
                result
            }
        }
        Impl {
            channel: &self.channel,
            f,
        }
    }
}

pub fn make_channel<Item>() -> (Sender<Item>, Receiver<Item>) {
    let channel = Arc::new(Mutex::new(Channel {
        items: Vec::new(),
        waker: None,
    }));
    (Sender { channel: channel.clone() }, Receiver { channel })
}

struct Wakeups {
    condvar: Condvar,
    woken: Mutex<Vec<usize>>,
}

pub struct Waker {
    wakeups: Arc<Wakeups>,
    key: usize,
}
impl Waker {
    pub fn wake(&self) {
        let mut woken = self.wakeups.woken.lock().unwrap();
        woken.push(self.key);
        self.wakeups.condvar.notify_all();
    }
}

pub struct Dispatch<'slf> {
    receivers: Vec<&'slf mut dyn Dispatchable>,
}
impl<'slf> Dispatch<'slf> {
    pub fn new() -> Self {
        Dispatch {
            receivers: Vec::new(),
        }
    }

    pub fn add(&mut self, receiver: &'slf mut dyn Dispatchable) {
        self.receivers.push(receiver);
    }

    pub fn wait_then_poll(mut self) {
        let wakeups = Arc::new(Wakeups {
            condvar: Condvar::new(),
            woken: Mutex::new(Vec::new()),
        });

        let mut ever_dispatched = self.do_poll(&wakeups, 0..self.receivers.len());
        let mut poll = Vec::new();

        loop {
            {
                let mut woken = wakeups.woken.lock().unwrap();

                if woken.is_empty() && !ever_dispatched {
                    woken = wakeups.condvar.wait(woken).unwrap();
                }

                std::mem::swap(&mut poll, &mut woken);
            }

            let dispatched = self.do_poll(&wakeups, poll.drain(..));
            ever_dispatched = ever_dispatched || dispatched;

            if !dispatched && ever_dispatched {
                break;
            }
        }
    }

    fn do_poll<I: Iterator<Item = usize>>(&mut self, wakeups: &Arc<Wakeups>, indices: I) -> bool {
        let mut waker = None;

        let mut ever_dispatched = false;

        for index in indices {
            waker.get_or_insert_with(|| Waker {
                wakeups: wakeups.clone(),
                key: index,
            }).key = index;

            while let Some(the_waker) = waker {
                waker = self.receivers[index].poll(the_waker);
                if waker.is_some() {
                    ever_dispatched = true;
                }
            }
        }

        ever_dispatched
    }
}
