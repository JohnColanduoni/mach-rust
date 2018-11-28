#![allow(bad_style)]
#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/mach.rs"));

#[cfg(feature = "mach_init")]
include!("mach_init.rs");

#[cfg(feature = "port")]
include!("port.rs");

#[cfg(feature = "message")]
include!("message.rs");
