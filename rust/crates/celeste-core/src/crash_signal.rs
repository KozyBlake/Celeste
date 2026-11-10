//! Writing one line to the log from inside the signal that is killing us.
//!
//! When native code below Rust decides the process is over -- Mesa calling
//! `abort()` after a GPU reset is the case this was written for -- no Rust
//! code runs by default. No panic hook, no unwind, no `Drop`. The log
//! simply stops, and the only account of the death comes from the *next*
//! run reading what was left behind.
//!
//! A signal handler changes that, within strict limits. SIGABRT is
//! catchable, and this is how crash reporters have always worked. What a
//! handler may do is the constraint: only async-signal-safe calls. No
//! allocating, no `format!`, no locking the logger's mutex -- any of which
//! can deadlock a process that is already dying, which is a far worse
//! outcome than a missing line.
//!
//! So everything that can be prepared is prepared at install time: the
//! file descriptor is already open, and every byte of the message except
//! two numbers is already laid out as a constant. At signal time this
//! writes those bytes, converts two integers into digits on the stack, and
//! re-raises. Nothing else.
//!
//! The line goes into the log Celeste is already writing, rather than a
//! file of its own, because that is where someone looks and because
//! [`self_crash`](crate::self_crash) reads the log's tail anyway -- so the
//! next run picks the line up with no second format to keep in step.

#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};

/// A second descriptor onto the log file, opened at install time.
///
/// Opening a file needs a path conversion and therefore an allocation,
/// which is exactly what a handler must not do -- so it happens once,
/// early, while it is still safe. `-1` means no handler is armed.
#[cfg(unix)]
static LOG_FD: AtomicI32 = AtomicI32::new(-1);

/// The parts of the line that never change, so the handler only has to
/// append digits to them.
#[cfg(unix)]
const PREFIX: &[u8] = b"\n*** FATAL: Celeste was killed by signal ";
#[cfg(unix)]
const MIDDLE: &[u8] = b" at unix time ";
#[cfg(unix)]
const SUFFIX: &[u8] =
    b" -- written by the crash handler; no further Celeste output follows.\n";

/// The signals that mean something below Rust ended the process.
///
/// `SIGABRT` is the one a lost GPU context produces. The rest are here
/// because a driver that is unhappy enough to abort is equally capable of
/// faulting, and a line in the log costs nothing in either case.
#[cfg(unix)]
const FATAL_SIGNALS: [libc::c_int; 5] = [
    libc::SIGABRT,
    libc::SIGSEGV,
    libc::SIGBUS,
    libc::SIGILL,
    libc::SIGFPE,
];

/// Arms the handler, writing into `log_file`.
///
/// Returns whether it armed. A failure here is never a reason to stop
/// starting: it costs one line in a crash that may not happen.
#[cfg(unix)]
pub fn install(log_file: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    if LOG_FD.load(Ordering::Relaxed) >= 0 {
        return true; // already armed; installing twice would leak the first fd
    }
    let Ok(path) = std::ffi::CString::new(log_file.as_os_str().as_bytes()) else {
        return false;
    };
    // O_APPEND is what makes this safe to share with the logger's own
    // handle: every write lands at the end as one atomic act, so this line
    // can never land in the middle of an event another thread is writing.
    // O_CLOEXEC keeps it out of any child process Celeste spawns.
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_APPEND | libc::O_CLOEXEC) };
    if fd < 0 {
        return false;
    }
    LOG_FD.store(fd, Ordering::Relaxed);

    for signal in FATAL_SIGNALS {
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            // The cast goes via a pointer: casting a function item
            // straight to an integer trips rustc's function_casts_as_integer
            // lint, and this tree is kept warning-free.
            action.sa_sigaction = on_fatal_signal as *const () as usize;
            // SA_RESETHAND makes the handler one-shot: by the time it
            // re-raises, the disposition is already back to the default, so
            // the process still dies with the right status and still dumps
            // core. A second fault inside the handler also dies rather than
            // recursing.
            action.sa_flags = libc::SA_RESETHAND;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
    true
}

/// No-op off unix: Windows has no signal disposition to install into, and
/// its fatal-error path is structured exceptions rather than signals.
#[cfg(not(unix))]
pub fn install(_log_file: &std::path::Path) -> bool {
    false
}

/// Everything below runs with the process already condemned. Every call
/// here is async-signal-safe: `write`, `time`, `raise`, and arithmetic.
#[cfg(unix)]
extern "C" fn on_fatal_signal(signal: libc::c_int) {
    let fd = LOG_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        write_all(fd, PREFIX);
        write_decimal(fd, signal as i64);
        write_all(fd, MIDDLE);
        // `time` is on POSIX's async-signal-safe list; `chrono` is not, and
        // a readable timestamp is not worth a deadlock. The next run turns
        // this into a date.
        write_decimal(fd, unsafe { libc::time(std::ptr::null_mut()) } as i64);
        write_all(fd, SUFFIX);
    }
    // The disposition is already SIG_DFL (SA_RESETHAND), so this ends the
    // process the way it would have ended without us -- same exit status,
    // same core dump.
    unsafe {
        libc::raise(signal);
    }
}

#[cfg(unix)]
fn write_all(fd: libc::c_int, mut buffer: &[u8]) {
    // A short write on a regular file is possible but vanishingly rare for
    // a line this size, and a handler must not loop indefinitely while the
    // process is dying: give up rather than hang.
    for _ in 0..8 {
        if buffer.is_empty() {
            return;
        }
        let written = unsafe { libc::write(fd, buffer.as_ptr().cast(), buffer.len()) };
        if written <= 0 {
            return;
        }
        buffer = &buffer[written as usize..];
    }
}

/// Writes `value` as decimal digits without allocating.
///
/// `format!` would allocate, which is the one thing a handler must not do,
/// so the digits are built backwards into a fixed stack buffer. Twenty
/// bytes holds any `i64`.
#[cfg(unix)]
fn write_decimal(fd: libc::c_int, value: i64) {
    let mut digits = [0u8; 20];
    let mut index = digits.len();
    let negative = value < 0;
    let mut remaining = value.unsigned_abs();
    loop {
        index -= 1;
        digits[index] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    if negative {
        index -= 1;
        digits[index] = b'-';
    }
    write_all(fd, &digits[index..]);
}

/// The name of a signal number, for a report to show beside the digits.
///
/// Lives here rather than in the handler: naming is for whoever reads the
/// line afterwards, and the handler cannot afford the lookup.
pub fn signal_name(signal: i64) -> Option<&'static str> {
    Some(match signal {
        4 => "SIGILL",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        15 => "SIGTERM",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_signals_a_report_will_show() {
        assert_eq!(signal_name(6), Some("SIGABRT"));
        assert_eq!(signal_name(11), Some("SIGSEGV"));
        assert_eq!(signal_name(9), Some("SIGKILL"));
        assert_eq!(signal_name(64), None, "unknown numbers stay numbers");
    }

    /// The handler writes digits by hand because it may not allocate.
    #[cfg(unix)]
    #[test]
    fn decimal_conversion_matches_the_formatter_it_replaces() {
        use std::io::{Read, Seek};
        for value in [0i64, 6, 9, 11, 1_794_275_000, i64::MAX, -1, i64::MIN] {
            let mut file = tempfile::tempfile().unwrap();
            let fd = std::os::unix::io::AsRawFd::as_raw_fd(&file);
            write_decimal(fd, value);
            file.rewind().unwrap();
            let mut written = String::new();
            file.read_to_string(&mut written).unwrap();
            assert_eq!(written, value.to_string(), "for {value}");
        }
    }
}
