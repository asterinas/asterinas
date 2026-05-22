// SPDX-License-Identifier: MPL-2.0

//! Error types for FUSE operations.

/// Error while encoding, decoding, or processing a FUSE message.
#[derive(Debug)]
pub enum FuseError {
    BufferTooSmall,
    LengthOverflow,
    MalformedResponse,
    PageFault,
    /// The FUSE server returned an error.
    RemoteError(i32),
    /// A resource allocation failed.
    ResourceAlloc(ostd::Error),
}

/// The result type used by FUSE operation encoders and decoders.
pub type FuseResult<T> = Result<T, FuseError>;
