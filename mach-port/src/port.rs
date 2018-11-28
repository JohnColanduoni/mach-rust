use crate::{RawPort, Msg};

use std::{io, mem, fmt};
use std::time::Duration;

use mach_sys as sys;
use mach_core::mach_call;

pub struct Port {
    port: sys::mach_port_name_t,
    has_receive: bool,
    has_send: bool,
}

impl Drop for Port {
    fn drop(&mut self) {
        unsafe {
            if self.has_receive {
                let _ = mach_call!(log: sys::mach_port_mod_refs(sys::mach_task_self(), self.port, sys::MACH_PORT_RIGHT_RECEIVE, -1), "freeing receive right with mach_port_mod_refs failed: {:?}");
            }
            if self.has_send {
                // If the receive right is already dead, this returns
                match sys::mach_port_mod_refs(sys::mach_task_self(), self.port, sys::MACH_PORT_RIGHT_SEND, -1) as u32 {
                    sys::KERN_SUCCESS | sys::KERN_INVALID_RIGHT => (),
                    code => {
                        let err = mach_core::error::rust_from_mach_error(code as _);
                        error!("freeing send right with mach_port_mod_refs failed: {:?}", err);
                    },
                }
            }
        }
    }
}

impl Port {
    pub fn new() -> io::Result<Port> {
        unsafe {
            let mut port: sys::mach_port_t = 0;
            mach_call!(log: sys::mach_port_allocate(sys::mach_task_self(), sys::MACH_PORT_RIGHT_RECEIVE, &mut port), "mach_port_allocate failed: {:?}")?;
            let port = Port {
                port,
                has_receive: true,
                has_send: false,
            };
            Ok(port)
        }
    }

    pub unsafe fn from_raw_port(port: RawPort) -> io::Result<Self> {
        let mut ty: sys::mach_port_type_t = 0;
        mach_call!(log: sys::mach_port_type(sys::mach_task_self(), port, &mut ty), "mach_port_type failed: {:?}")?;
        // TODO: support send-once

        Ok(Port {
            port,
            has_send: ty & sys::MACH_PORT_TYPE_SEND != 0,
            has_receive: ty & sys::MACH_PORT_TYPE_RECEIVE != 0,
        })
    }

    pub fn as_raw_port(&self) -> RawPort {
        self.port
    }

    pub fn into_raw_port(self) -> RawPort {
        let port = self.port;
        mem::forget(self);
        port
    }

    pub fn make_sender(&self) -> io::Result<Port> {
        unsafe {
            let mut port: sys::mach_port_t = 0;
            let mut right: sys::mach_msg_type_name_t = 0;
            mach_call!(log: sys::mach_port_extract_right(sys::mach_task_self(), self.port, sys::MACH_MSG_TYPE_MAKE_SEND, &mut port, &mut right), "mach_port_extract_right failed: {:?}")?;
            if right != sys::MACH_MSG_TYPE_PORT_SEND {
                return Err(io::Error::new(io::ErrorKind::Other, "mach_port_extract_right did not return requested right type"));
            }
            let port = Port {
                port,
                has_receive: false,
                has_send: true,
            };
            Ok(port)
        }
    }

    pub fn send(&self, msg: &mut Msg, timeout: Option<Duration>) -> io::Result<()> {
        unsafe {
            let mut flags = sys::MACH_SEND_MSG;
            let mut timeout_arg = sys::MACH_MSG_TIMEOUT_NONE as sys::mach_msg_timeout_t;
            if let Some(duration) = timeout {
                flags |= sys::MACH_RCV_TIMEOUT;
                timeout_arg = convert_timeout(duration);
            }
            msg.header_mut().msgh_remote_port = self.port;
            let result = mach_call!(sys::mach_msg(
                msg.0.as_ptr() as *mut _,
                flags as _,
                msg.header().msgh_size,
                0,
                sys::MACH_PORT_NULL,
                timeout_arg,
                sys::MACH_PORT_NULL,
            ));
            msg.header_mut().msgh_remote_port = sys::MACH_PORT_NULL;
            result?;
            msg.0.reset_on_send();
            Ok(())
        }
    }

    pub fn recv(&self, msg: &mut Msg, timeout: Option<Duration>) -> io::Result<()> {
        unsafe {
            let mut flags = sys::MACH_RCV_MSG | sys::MACH_RCV_LARGE;
            let mut timeout_arg = sys::MACH_MSG_TIMEOUT_NONE as sys::mach_msg_timeout_t;
            if let Some(duration) = timeout {
                flags |= sys::MACH_RCV_TIMEOUT;
                timeout_arg = convert_timeout(duration);
            }
            mach_call!(sys::mach_msg(
                msg.0.as_mut_ptr() as *mut _,
                flags as _,
                0,
                msg.0.capacity() as _,
                self.port,
                timeout_arg,
                sys::MACH_PORT_NULL,
            ))?;

            let size = msg.header().msgh_size;
            msg.0.set_len(size as usize);

            Ok(())
        }
    }
}

impl fmt::Debug for Port {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Port")
            .field("port", &format_args!("{:#x?}", self.port))
            .field("has_receive", &self.has_receive)
            .field("has_send", &self.has_send)
            .finish()
    }
}

fn convert_timeout(duration: Duration) -> sys::mach_msg_timeout_t {
    duration
        .as_secs()
        .checked_mul(1000)
        .and_then(|x| x.checked_add(duration.subsec_millis() as u64))
        .filter(|&x| x <= std::i32::MAX as u64)
        .map(|x| x as i32)
        .unwrap_or(std::i32::MAX) as sys::mach_msg_timeout_t
}