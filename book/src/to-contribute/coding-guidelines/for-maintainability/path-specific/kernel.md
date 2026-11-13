# `kernel/`

Guidelines that apply only to code under `kernel/`.

## Use `errno!` to construct errors (`use-errno-macro`) {#use-errno-macro}

Prefer `errno!` over `Error::new` when constructing kernel errors,
as it is more concise:

```rust
// Good
return Err(errno!(EINVAL));

// Bad
return Err(Error::new(Errno::EINVAL));
```

To minimize its memory footprint, `Error` cannot carry an error message.
However, you can document an error by including a message in `errno!`:

```rust
// Even better
return Err(errno!(EINVAL, "the file offset is not page-aligned"));
```

The message serves only as documentation;
it is not compiled into the binary and therefore incurs no runtime overhead.

See also:
[`error-message-format`](../naming.md#error-message-format).

### Track callers when converting errors (`track-error-caller`) {#track-error-caller}

The kernel `Error` type records the caller's source location when it is constructed
and preserves that location as the error propagates.
This helps developers trace a failure back to its origin,
even when the error is reported far from where it occurred.

To capture the caller's location when converting another error type into a kernel `Error`,
add `#[track_caller]` to every function involved in the conversion,
including `From::from` implementations and intermediate conversion helpers.
Without the attribute, the recorded location points inside the conversion function
instead of the caller that encountered the error.

```rust
impl From<core::str::Utf8Error> for Error {
    #[track_caller]
    fn from(_: core::str::Utf8Error) -> Self {
        errno!(EINVAL, "invalid UTF-8 string")
    }
}
```

See also:
[Rust Reference: The `track_caller` attribute](https://doc.rust-lang.org/reference/attributes/codegen.html#the-track_caller-attribute).
