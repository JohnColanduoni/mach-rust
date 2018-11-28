pub const MACH_PORT_DEAD: mach_port_name_t = !0;

pub const MACH_PORT_RIGHT_SEND: mach_port_right_t = 0;
pub const MACH_PORT_RIGHT_RECEIVE: mach_port_right_t = 1;

pub const MACH_PORT_TYPE_SEND: mach_port_type_t = MACH_PORT_TYPE(MACH_PORT_RIGHT_SEND);
pub const MACH_PORT_TYPE_RECEIVE: mach_port_type_t = MACH_PORT_TYPE(MACH_PORT_RIGHT_RECEIVE);

const fn MACH_PORT_TYPE(right: mach_port_right_t) -> mach_port_type_t {
    1 << (right + 16)
}