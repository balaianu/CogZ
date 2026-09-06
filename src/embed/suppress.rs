//! Stderr suppression during ONNX Runtime initialization.
//!
//! The ONNX Runtime C++ library emits ~1258 duplicate schema
//! registration warnings directly to stderr (fd 2) during init.
//! These are harmless (idempotent re-registration) but flood
//! terminal output. This module provides a helper to temporarily
//! redirect stderr to /dev/null during a closure call.

/// Run a closure with stderr (fd 2) redirected to /dev/null, then
/// restore the original stderr.
///
/// On non-Unix platforms this is a no-op passthrough.
#[cfg(unix)]
pub fn suppress_stderr_during<F, T>(f: F) -> T
where
    F: FnOnce() -> T,
{
    use std::os::unix::io::AsRawFd;

    // Hold the stderr lock to prevent concurrent writes during redirection.
    let _stderr = std::io::stderr();
    let saved = unsafe { libc::dup(libc::STDERR_FILENO) };
    if saved < 0 {
        return f();
    }

    let null = std::fs::File::open("/dev/null").unwrap();
    let _ = unsafe { libc::dup2(null.as_raw_fd(), libc::STDERR_FILENO) };
    drop(null);

    let result = f();

    let _ = unsafe { libc::dup2(saved, libc::STDERR_FILENO) };
    unsafe { libc::close(saved) };

    result
}

#[cfg(not(unix))]
pub fn suppress_stderr_during<F, T>(f: F) -> T
where
    F: FnOnce() -> T,
{
    f()
}
