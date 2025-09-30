# Notes about implementation and design decisions

## Use of `tokio::sync::watch`

Instead of protecting access to variables with Mutex/Locks, `tokio::sync::watch` allows consumers to access the
"latest" data without waiting on any locks so it is used in most of the hot paths.

I don't know if this is idiomatic or best practice but it seems reasonable?