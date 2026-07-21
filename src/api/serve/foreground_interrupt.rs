use std::io;

/// Keeps terminal-generated interrupts available while foreground Web runs.
///
/// A TUI or an abruptly terminated terminal program can leave the controlling
/// terminal in raw mode with `ISIG` disabled. In that state Ctrl+C is delivered
/// as the byte `0x03` instead of `SIGINT`, so Tokio's signal listener never
/// wakes. Foreground Web promises Ctrl+C shutdown, therefore it temporarily
/// enables only the terminal's signal-generation flag and restores the caller's
/// original mode when the command returns.
pub(super) struct InterruptSignalGuard {
    #[cfg(unix)]
    terminal: Option<UnixTerminalMode>,
}

#[cfg(unix)]
struct UnixTerminalMode {
    fd: std::os::fd::RawFd,
    original: libc::termios,
}

impl InterruptSignalGuard {
    pub(super) fn enable() -> io::Result<Self> {
        #[cfg(unix)]
        {
            Self::enable_for_fd(libc::STDIN_FILENO)
        }

        #[cfg(not(unix))]
        {
            Ok(Self {})
        }
    }

    #[cfg(unix)]
    fn enable_for_fd(fd: std::os::fd::RawFd) -> io::Result<Self> {
        if unsafe { libc::isatty(fd) } != 1 {
            return Ok(Self { terminal: None });
        }

        let original = read_terminal_mode(fd)?;
        if original.c_lflag & libc::ISIG != 0 {
            return Ok(Self { terminal: None });
        }

        let mut interruptible = original;
        interruptible.c_lflag |= libc::ISIG;
        write_terminal_mode(fd, &interruptible)?;
        Ok(Self {
            terminal: Some(UnixTerminalMode { fd, original }),
        })
    }
}

#[cfg(unix)]
impl Drop for InterruptSignalGuard {
    fn drop(&mut self) {
        if let Some(terminal) = self.terminal.take() {
            if let Err(error) = write_terminal_mode(terminal.fd, &terminal.original) {
                tracing::warn!(%error, "failed to restore terminal interrupt mode");
            }
        }
    }
}

#[cfg(unix)]
fn read_terminal_mode(fd: std::os::fd::RawFd) -> io::Result<libc::termios> {
    let mut mode = std::mem::MaybeUninit::uninit();
    if unsafe { libc::tcgetattr(fd, mode.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { mode.assume_init() })
}

#[cfg(unix)]
fn write_terminal_mode(fd: std::os::fd::RawFd, mode: &libc::termios) -> io::Result<()> {
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, mode) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::fd::RawFd;

    struct TestPty {
        master: RawFd,
        slave: RawFd,
    }

    impl TestPty {
        fn open() -> Self {
            let mut master = -1;
            let mut slave = -1;
            let result = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            assert_eq!(result, 0, "openpty failed: {}", io::Error::last_os_error());
            Self { master, slave }
        }
    }

    impl Drop for TestPty {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.master);
                libc::close(self.slave);
            }
        }
    }

    #[test]
    fn enables_and_restores_interrupt_generation_for_raw_terminal() {
        let pty = TestPty::open();
        let mut raw = read_terminal_mode(pty.slave).expect("read pseudo-terminal mode");
        raw.c_lflag &= !libc::ISIG;
        write_terminal_mode(pty.slave, &raw).expect("disable pseudo-terminal interrupts");

        let guard = InterruptSignalGuard::enable_for_fd(pty.slave)
            .expect("enable pseudo-terminal interrupts");
        let enabled = read_terminal_mode(pty.slave).expect("read enabled terminal mode");
        assert_eq!(enabled.c_lflag, raw.c_lflag | libc::ISIG);
        assert_eq!(enabled.c_iflag, raw.c_iflag);
        assert_eq!(enabled.c_oflag, raw.c_oflag);
        assert_eq!(enabled.c_cflag, raw.c_cflag);
        assert_eq!(enabled.c_cc, raw.c_cc);
        assert_eq!(unsafe { libc::cfgetispeed(&enabled) }, unsafe {
            libc::cfgetispeed(&raw)
        });
        assert_eq!(unsafe { libc::cfgetospeed(&enabled) }, unsafe {
            libc::cfgetospeed(&raw)
        });

        drop(guard);
        let restored = read_terminal_mode(pty.slave).expect("read restored terminal mode");
        assert_eq!(restored.c_lflag, raw.c_lflag);
        assert_eq!(restored.c_iflag, raw.c_iflag);
        assert_eq!(restored.c_oflag, raw.c_oflag);
        assert_eq!(restored.c_cflag, raw.c_cflag);
        assert_eq!(restored.c_cc, raw.c_cc);
        assert_eq!(unsafe { libc::cfgetispeed(&restored) }, unsafe {
            libc::cfgetispeed(&raw)
        });
        assert_eq!(unsafe { libc::cfgetospeed(&restored) }, unsafe {
            libc::cfgetospeed(&raw)
        });
    }

    #[test]
    fn non_terminal_file_descriptor_is_a_no_op() {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let guard = InterruptSignalGuard::enable_for_fd(fds[0])
            .expect("non-terminal descriptors should not fail");
        assert!(guard.terminal.is_none());
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }
}
