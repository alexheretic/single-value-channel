single_value_channel
[![crates.io](https://img.shields.io/crates/v/single_value_channel.svg)](https://crates.io/crates/single_value_channel)
[![Documentation](https://docs.rs/single_value_channel/badge.svg)](https://docs.rs/single_value_channel")
================

Non-blocking single value update and receive channel.

This module provides a latest-message style channel, where update sources can update the latest value that the
receiver owns in a practically non-blocking way.

Unlike the `mpsc::channel` each value send will overwrite the 'latest' value. See the [documentation](https://docs.rs/single_value_channel) for
more details.

```rust
use single_value_channel::channel_starting_with;
use std::thread;

let (mut receiver, updater) = channel_starting_with(0);
assert_eq!(*receiver.latest(), 0);

thread::spawn(move|| {
    updater.update(2); // next access to receiver.latest() -> 2
    updater.update(12); // next access to receiver.latest() -> 12
}).join();

assert_eq!(*receiver.latest(), 12);
```
