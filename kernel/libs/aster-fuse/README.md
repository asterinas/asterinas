# aster-fuse

`aster-fuse` is the shared FUSE protocol crate for Asterinas.

It provides the common protocol vocabulary used by in-kernel filesystem clients
that talk to FUSE-compatible servers. Keeping these definitions in one crate
allows clients to share the same request and reply formats instead of maintaining
their own copies.

## Scope

This crate only describes the FUSE protocol. It does not choose a transport,
submit requests, manage queues, or own I/O buffers. Those responsibilities
belong to the filesystem client built on top of it.
