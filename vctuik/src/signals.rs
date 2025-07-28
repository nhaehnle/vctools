use std::sync::{Arc, Condvar, Mutex};

/// A `Dispatchable` is an object that handles an asynchronous trigger.
///
/// The trigger originates in a different thread from where the handler runs.
pub trait Dispatchable {
    /// `poll` is called on the thread where the handler should run.
    ///
    /// `poll` must run whatever the Dispatchable implementation wants to do on the handler thread
    /// based on the asynchronous trigger.
    ///
    /// If there is no more work to be done and a `waker` is provided, `poll` must take ownership
    /// of the waker object. The Dispatchable implementation must then ensure that the waker
    /// object's `Waker::wake` method is called once more work is to be done. (Otherwise, there is
    /// no guarantee that `poll` will ever be called again.)
    ///
    /// Typically, `Waker::wake` is called from a different thread.
    fn poll(&mut self, waker: &mut Option<Waker>);
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
            fn poll(&mut self, waker: &mut Option<Waker>) {
                let items = {
                    let mut channel = self.channel.lock().unwrap();
                    if channel.items.is_empty() {
                        channel.waker = waker.take();
                        Vec::new()
                    } else {
                        std::mem::replace(&mut channel.items, Vec::new())
                    }
                };
                for item in items {
                    (self.f)(item);
                }
            }
        }
        Impl {
            channel: &self.channel,
            f,
        }
    }

    pub fn dispatch_one<'recv, F>(&'recv mut self, f: F) -> impl Dispatchable + 'recv
    where
        F: FnOnce(Item) + 'recv
    {
        struct Impl<'recv, Item, F> {
            channel: &'recv Mutex<Channel<Item>>,
            f: Option<F>,
        }
        impl<'recv, Item, F: FnOnce(Item) + 'recv> Dispatchable for Impl<'recv, Item, F> {
            fn poll(&mut self, waker: &mut Option<Waker>) {
                if self.f.is_none() {
                    // We have already dispatched once; there is nothing left to do and we will
                    // never wake up.
                    std::mem::drop(waker.take());
                    return;
                }

                if let Some(item) = {
                    let mut channel = self.channel.lock().unwrap();
                    if channel.items.is_empty() {
                        channel.waker = waker.take();
                        None
                    } else {
                        Some(channel.items.remove(0))
                    }
                } {
                    (self.f.take().unwrap())(item);
                }
            }
        }
        Impl {
            channel: &self.channel,
            f: Some(f),
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
    receivers: Vec<Box<dyn Dispatchable + 'slf>>,
}
impl<'slf> Dispatch<'slf> {
    pub fn new() -> Self {
        Dispatch {
            receivers: Vec::new(),
        }
    }

    pub fn add(&mut self, receiver: impl Dispatchable + 'slf) {
        self.receivers.push(Box::new(receiver));
    }

    /// Dispatches the dispatchables
    ///
    /// Polls every dispatchable at least once.
    ///
    /// If `wait` is true, we block once until at least one dispatchable was dispatched.
    pub fn poll(mut self, wait: bool) {
        let wakeups = wait.then(|| Arc::new(Wakeups {
            condvar: Condvar::new(),
            woken: Mutex::new(Vec::new()),
        }));

        let all_wait = self.do_poll(&wakeups, 0..self.receivers.len());

        if let Some(wakeups) = wakeups {
            if all_wait {
                let mut poll = Vec::new();

                {
                    let mut woken = wakeups.woken.lock().unwrap();

                    if woken.is_empty() {
                        woken = wakeups.condvar.wait(woken).unwrap();
                    }

                    std::mem::swap(&mut poll, &mut woken);
                }

                self.do_poll(&None, poll.drain(..));
            }
        }
    }

    /// Poll the dispatchables with the given indices.
    ///
    /// Wakers for dispatchable that need to wait are registered in `wakeups` if provided.
    ///
    /// If `wakeups` is provided, returns true if all dispatchables have to wait. (If `wakeups` is
    /// not provided, the return value is meaningless.)
    fn do_poll<I: Iterator<Item = usize>>(&mut self, wakeups: &Option<Arc<Wakeups>>, indices: I) -> bool {
        let mut waker = None;
        let mut all_wait = true;

        for index in indices {
            if let Some(wakeups) = wakeups.as_ref() {
                waker.get_or_insert_with(|| Waker {
                    wakeups: wakeups.clone(),
                    key: index,
                }).key = index;
            }

            self.receivers[index].poll(&mut waker);

            if waker.is_some() {
                all_wait = false;
            }
        }

        all_wait
    }
}
