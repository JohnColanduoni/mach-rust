use std::{io, fmt};
use std::ffi::CStr;

use mach_sys as sys;

#[macro_export]
macro_rules! mach_call {
    (log: $x:expr, $fmt_str:tt $(, $fmt_arg:expr $(,)*)* ) => {
        match mach_call!($x) {
            Ok(()) => Ok(()),
            Err(err) => {
                ::log::error!($fmt_str, err, $($fmt_arg,)* );
                Err(err)
            }
        }
    };
    ($x:expr) => {
        match $x {
            0 => Ok(()),
            code => {
                let err = $crate::error::rust_from_mach_error(code);
                Err(err)
            }
        }
    };
}

#[macro_export]
macro_rules! mach_kern_call {
    (log: $x:expr, $fmt_str:tt $(, $fmt_arg:expr $(,)*)* ) => {
        match mach_kern_call!($x) {
            Ok(()) => Ok(()),
            Err(err) => {
                ::log::error!($fmt_str, err, $($fmt_arg,)* );
                Err(err)
            }
        }
    };
    ($x:expr) => {
        match $x {
            0 => Ok(()),
            code => {
                let err = $crate::error::rust_from_mach_kern_error(code);
                Err(err)
            }
        }
    };
}

pub fn rust_from_mach_error(code: sys::mach_error_t) -> io::Error {
    // TODO: transfer more equivalent codes to io::ErrorKind
    let kind = match code as u32 {
        sys::MACH_SEND_TIMED_OUT => io::ErrorKind::TimedOut,
        sys::MACH_RCV_TIMED_OUT => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, ErrorWrapper(code))
}

pub fn rust_from_mach_kern_error(code: sys::kern_return_t) -> io::Error {
    // TODO: transfer equivalent codes to io::ErrorKind
    io::Error::new(io::ErrorKind::Other, KernErrorWrapper(code))
}


// Struct that wraps a mach error code for placement inside a std::io::Error
struct ErrorWrapper(sys::mach_error_t);

impl fmt::Debug for ErrorWrapper {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let name = unsafe { CStr::from_ptr(sys::mach_error_string(self.0)) };
        write!(f, "MachError {{ code: {:#x?}, description: {:?} }}", self.0, name)
    }
}

impl fmt::Display for ErrorWrapper {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let name = unsafe { CStr::from_ptr(sys::mach_error_string(self.0)) };
        write!(f, "{:?}", name)
    }
}

impl std::error::Error for ErrorWrapper {

}

// Struct that wraps a mach error code for placement inside a std::io::Error
struct KernErrorWrapper(sys::mach_error_t);

impl fmt::Debug for KernErrorWrapper {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MachKernError {{ code: {:#x?} }}", self.0)
    }
}

impl fmt::Display for KernErrorWrapper {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: string values, copy from IOService.h
        write!(f, "(code {:#x?})", self.0)
    }
}

impl std::error::Error for KernErrorWrapper {

}