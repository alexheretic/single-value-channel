//! Single value update and receive channel.
//!
//! This module provides a latest-message style channel, where update sources can update the
//! latest value that the receiver owns in a practically non-blocking way.
//!
//! Unlike the `mpsc::channel` each value send will overwrite the 'latest' value.
//!
//! This is useful, for example, when thread ***A*** is interested in the latest result of another
//! continually working thread ***B***. ***B*** could update the channel with it's latest result
//! each iteration, each new update becoming the new 'latest' value. Then ***A*** can access
//! that latest value. Neither the write nor read are blocked by the other thread.
//!
//! It is practically non-blocking as an updater thread cannot block the receiver thread or
//! vice versa. The internal sync primitives are private and essentially only lock over fast data
//! moves.
//!
//! # Example
//! ```
//! use single_value_channel::channel_starting_with;
//! use std::thread;
//!
//! let (mut receiver, updater) = channel_starting_with(0);
//! assert_eq!(*receiver.latest(), 0);
//!
//! thread::spawn(move || {
//!     updater.update(2); // next access to receiver.latest() -> 2
//!     updater.update(12); // next access to receiver.latest() -> 12
//! })
//! .join();
//!
//! assert_eq!(*receiver.latest(), 12);
//! ```

use std::result::Result;
use std::sync::{Arc, Mutex, Weak};

/// The receiving-half of the single value channel.
#[derive(Debug)]
pub struct Receiver<T> {
    latest: T,
    latest_set: Arc<Mutex<Option<T>>>,
}

impl<T> Receiver<T> {
    fn update_latest(&mut self) {
        if let Ok(mut latest_set) = self.latest_set.lock() {
            if let Some(value) = latest_set.take() {
                self.latest = value;
            }
        }
    }

    /// Access latest updated value
    pub fn latest(&mut self) -> &T {
        self.update_latest();
        &self.latest
    }

    /// Access latest updated value mutably
    pub fn latest_mut(&mut self) -> &mut T {
        self.update_latest();
        &mut self.latest
    }

    /// Returns true if the all related `Updater` instances have been dropped.
    pub fn has_no_updater(&self) -> bool {
        Arc::weak_count(&self.latest_set) == 0
    }
}

/// The updating-half of the single value channel.
#[derive(Debug, Clone)]
pub struct Updater<T> {
    latest: Weak<Mutex<Option<T>>>,
}

/// An error returned from the [`Updater::update`](struct.Updater.html#method.update) function.
/// Indicates that the paired [`Receiver`](struct.Receiver.html) has been dropped.
///
/// Contains the value that had been passed into
/// [`Updater::update`](struct.Updater.html#method.update)
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct NoReceiverError<T>(pub T);

impl<T> Updater<T> {
    /// Updates the latest value in this channel, to be accessed the next time
    /// [`Receiver::latest`](struct.Receiver.html#method.latest) or
    /// [`Receiver::latest_mut`](struct.Receiver.html#method.latest_mut) is called.
    ///
    /// This call will fail with [`NoReceiverError`](struct.NoReceiverError.html) if the receiver
    /// has been dropped.
    pub fn update(&self, value: T) -> Result<(), NoReceiverError<T>> {
        match self.latest.upgrade() {
            Some(mutex) => {
                *mutex.lock().unwrap() = Some(value);
                Ok(())
            }
            None => Err(NoReceiverError(value)),
        }
    }

    /// Returns true if the receiver has been dropped. Thus indicating any following call to
    /// [`Updater::update`](struct.Updater.html#method.update) would fail.
    pub fn has_no_receiver(&self) -> bool {
        self.latest.upgrade().is_none()
    }
}

/// Constructs a single value channel with an initial value. Thus initial calls to
/// [`Receiver::latest`](struct.Receiver.html#method.latest) will return that value until
/// a [`Updater::update`](struct.Updater.html#method.update) call replaces the latest value.
pub fn channel_starting_with<T>(initial: T) -> (Receiver<T>, Updater<T>) {
    let receiver = Receiver {
        latest: initial,
        latest_set: Arc::new(Mutex::new(None)),
    };
    let updater = Updater {
        latest: Arc::downgrade(&receiver.latest_set),
    };
    (receiver, updater)
}

/// Constructs a single value channel. Initial calls to
/// [`Receiver::latest`](struct.Receiver.html#method.latest) will return `None`.
///
/// Since the initial value is `None` all calls to
/// [`Updater::update`](struct.Updater.html#method.update) must be wrapped in an option.
/// To avoid this consider providing an initial value to the channel with
/// [`channel_starting_with`](fn.channel_starting_with.html)
pub fn channel<T>() -> (Receiver<Option<T>>, Updater<Option<T>>) {
    channel_starting_with(None)
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::Barrier;
    use std::{mem, thread};

    #[test]
    fn send_recv_value() {
        let (mut recv, send) = channel_starting_with(12);
        assert_eq!(recv.latest(), &12);
        send.update(123).unwrap();
        assert_eq!(recv.latest(), &123);
    }

    #[test]
    fn send_recv_option() {
        // ensure option value works nicely
        let (mut recv, send) = channel_starting_with(None);
        assert_eq!(*recv.latest(), None);
        send.update(Some(234)).unwrap();
        assert_eq!(*recv.latest(), Some(234));
    }

    fn barrier_pair() -> (Arc<Barrier>, Arc<Barrier>) {
        let barrier = Arc::new(Barrier::new(2));
        (barrier.clone(), barrier)
    }

    #[test]
    fn concurrent_send_recv() {
        let (mut recv, send) = channel_starting_with(0);
        let (barrier, barrier2) = barrier_pair();

        thread::spawn(move || {
            barrier2.wait(); // <- read initial
            for num in 1..1000 {
                send.update(num).unwrap();
            }
            send.update(1000).unwrap();

            barrier2.wait(); // <- sent 1000
            for num in 1001..2001 {
                send.update(num).unwrap();
            }
            barrier2.wait(); // <- sent 2000
        });

        let mut distinct_recvs = 1;
        let mut last_result = *recv.latest();
        barrier.wait(); // <- read initial
        while last_result < 1000 {
            let next = *recv.latest();
            if next != last_result {
                distinct_recvs += 1;
            }
            last_result = next;
        }
        assert!(distinct_recvs > 1);
        println!("received: {}", distinct_recvs);

        assert_eq!(*recv.latest(), 1000);
        barrier.wait(); // <- sent 1000
        barrier.wait(); // <- sent 2000
        assert_eq!(*recv.latest(), 2000);
    }

    #[test]
    fn non_blocking_write_during_read() {
        let (mut name_get, name) = channel_starting_with("Nothing".to_owned());
        let (barrier, barrier2) = barrier_pair();
        thread::spawn(move || {
            barrier2.wait(); // <- has read lock
            name.update("Something".to_owned()).unwrap();
            barrier2.wait(); // <- value updated
        });

        {
            let got = name_get.latest();
            assert_eq!(*got, "Nothing".to_owned());
            barrier.wait(); // <- has read lock
            barrier.wait(); // <- value updated
        }

        let got2 = name_get.latest();
        assert_eq!(*got2, "Something".to_owned());
    }

    #[test]
    fn error_writing_to_dead_reader() {
        let (val_get, val) = channel_starting_with(0);
        mem::drop(val_get);
        assert_eq!(val.update(123), Err(NoReceiverError(123)));
    }

    #[test]
    fn updater_has_no_receiver() {
        let (receiver, updater) = channel_starting_with(0);
        assert!(!updater.has_no_receiver());

        mem::drop(receiver);
        assert!(updater.has_no_receiver());
    }

    #[test]
    fn receiver_has_no_updater() {
        let (receiver, updater) = channel_starting_with(0);
        assert!(!receiver.has_no_updater());

        let updater2 = updater.clone();
        assert!(!receiver.has_no_updater());

        mem::drop(updater);
        assert!(!receiver.has_no_updater());

        mem::drop(updater2);
        assert!(receiver.has_no_updater());
    }

    #[test]
    fn latest_mut() {
        let (mut val_get, _) = channel_starting_with("".to_owned());
        {
            val_get.latest_mut().push_str("hello");
        }
        assert_eq!(val_get.latest(), "hello");
    }

    #[test]
    fn multiple_updaters() {
        let (mut val_get, val1) = channel_starting_with(0);
        let val2 = val1.clone();

        val1.update(2).unwrap();
        assert_eq!(*val_get.latest(), 2);

        val2.update(3).unwrap();
        assert_eq!(*val_get.latest(), 3);
    }

    #[test]
    fn no_args_channel() {
        let (mut val_get, val) = channel();
        assert_eq!(*val_get.latest(), None);
        val.update(Some(123)).unwrap();
        assert_eq!(*val_get.latest(), Some(123));
    }
}
