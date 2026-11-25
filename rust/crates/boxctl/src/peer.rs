use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

pub fn is_authorized(uid: u32) -> bool {
    uid == 0
}

pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        // SAFETY: the file descriptor is valid for the lifetime of `stream`;
        // `credentials` and `length` point to correctly sized, writable storage.
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast::<libc::c_void>(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected peer credential size",
        ));
    }
    Ok(credentials.uid)
}

#[cfg(test)]
mod tests {
    use super::is_authorized;

    #[test]
    fn only_root_is_authorized() {
        assert!(is_authorized(0));
        assert!(!is_authorized(1000));
        assert!(!is_authorized(u32::MAX));
    }
}
