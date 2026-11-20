# For Documentation

*Are user-facing docs and compatibility artifacts correct, current, and well-written?*

This is the index of the **documentation** guidelines.
Each subsection is its own page,
and each entry below links a stable `short-name` to its guideline,
with a one-line gist so a reader (or a review tool) can grasp the guideline before opening it.

## Index

**[General Style](general-style.md)**
- [`semantic-line-breaks`](general-style.md#semantic-line-breaks): Break prose at sentence/clause boundaries, one idea per line.
- [`readme-as-crate-doc`](general-style.md#readme-as-crate-doc): For a published crate, make `README.md` the crate-level doc (`#![doc = include_str!("../README.md")]`) so crates.io and docs.rs stay in sync.

**[Path-Specific](path-specific/)**
- [`kernel/`](path-specific/kernel.md)
    - [`linux-compat-docs`](path-specific/kernel.md#linux-compat-docs): Update Linux Compatibility docs upon changes to Linux feature coverage, e.g., system calls, file systems, sockets, boot protocols, and kernel parameters.
