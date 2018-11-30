use crate::{Port, RawPort};

use std::{io, mem, ptr, slice, fmt};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use mach_sys as sys;

pub struct MsgBuffer {
    buffer: Vec<u8>,
    capacity_inline: usize,
    capacity_descriptors: usize,
}

impl Drop for MsgBuffer {
    fn drop(&mut self) {
        // FIXME: we should deallocate all MOVE ports and memory regions
    }
}

pub struct Msg(pub(crate) dyn MsgImpl);

#[repr(C)]
pub struct MsgDescriptor(sys::mach_msg_type_descriptor_t);

#[repr(C)]
pub struct MsgPortDescriptor(sys::mach_msg_port_descriptor_t);

pub enum MsgDescriptorKind<'a> {
    Port(&'a MsgPortDescriptor),
    // TODO: other subtypes
    Ool(&'a MsgDescriptor),
    OolPorts(&'a MsgDescriptor),
    OolVolatile(&'a MsgDescriptor),
}

pub enum MsgDescriptorKindMut<'a> {
    Port(&'a mut MsgPortDescriptor),
    // TODO: other subtypes
    Ool(&'a mut MsgDescriptor),
    OolPorts(&'a mut MsgDescriptor),
    OolVolatile(&'a mut MsgDescriptor),
}

pub(crate) trait MsgImpl {
    fn as_ptr(&self) -> *const u8;
    fn as_mut_ptr(&mut self) -> *mut u8;

    fn len(&self) -> usize;
    fn capacity(&self) -> usize;
    unsafe fn set_len(&mut self, len: usize);

    fn reset_on_send(&mut self);
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PortMoveMode {
    Receive,
    Send,
    SendOnce,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PortCopyMode {
    Send,
    MakeSend,
    MakeSendOnce,
}

#[repr(C)]
struct MessageStart {
    header: sys::mach_msg_header_t,
    body: sys::mach_msg_body_t,
}

// FIXME: a ton of these calculations probably need checked arithmetic in release (e.g. trying to put over 4GB of data in
// a message)
impl MsgBuffer {
    pub fn new() -> MsgBuffer {
        // Always keep enough additional capacity around for the trailer, in case we use this buffer for a receive
        let init_len = mem::size_of::<MessageStart>();
        let mut buffer = Vec::with_capacity(init_len + mem::size_of::<sys::mach_msg_trailer_t>());
        unsafe {
            *(buffer.as_mut_ptr() as *mut MessageStart) = MessageStart {
                header: sys::mach_msg_header_t {
                    msgh_bits: sys::MACH_MSG_TYPE_COPY_SEND,
                    msgh_size: mem::size_of::<MessageStart>() as _,
                    msgh_remote_port: sys::MACH_PORT_NULL,
                    msgh_local_port: sys::MACH_PORT_NULL,
                    msgh_voucher_port: sys::MACH_PORT_NULL,
                    msgh_id: 0,
                },
                body: sys::mach_msg_body_t {
                    msgh_descriptor_count: 0,
                },
            };
            buffer.set_len(init_len);
        }
        MsgBuffer {
            buffer,
            capacity_inline: 0,
            capacity_descriptors: 0,
        }
    }

    /// Resets the [`MsgBuffer`], deallocating any owned resources contained within.
    pub fn reset(&mut self) {
        debug_assert!(self.buffer.len() >= mem::size_of::<MessageStart>());
        unsafe {
            self.buffer.set_len(mem::size_of::<MessageStart>());
            *(self.buffer.as_mut_ptr() as *mut MessageStart) = MessageStart {
                header: sys::mach_msg_header_t {
                    msgh_bits: sys::MACH_MSG_TYPE_COPY_SEND,
                    msgh_size: mem::size_of::<MessageStart>() as _,
                    msgh_remote_port: sys::MACH_PORT_NULL,
                    msgh_local_port: sys::MACH_PORT_NULL,
                    msgh_voucher_port: sys::MACH_PORT_NULL,
                    msgh_id: 0,
                },
                body: sys::mach_msg_body_t {
                    msgh_descriptor_count: 0,
                },
            };
        }
        // FIXME: we should deallocate all MOVE port rights and memory regions
    }

    #[inline]
    pub fn reserve_inline_data(&mut self, additional: usize) {
        if self.capacity_inline < self.inline_data().len() + additional {
            self.capacity_inline = self.inline_data().len() + additional;
            self.update_reservation();
        }
    }

    #[inline]
    pub fn reserve_descriptors(&mut self, additional: usize) {
        if self.capacity_descriptors < self.descriptors().len() + additional {
            self.capacity_descriptors = self.descriptors().len() + additional;
            self.update_reservation();
        }
    }

    fn update_reservation(&mut self) {
        let total_capacity = mem::size_of::<MessageStart>() + self.capacity_descriptors * mem::size_of::<sys::mach_msg_descriptor_t>() + self.capacity_inline + mem::size_of::<sys::mach_msg_trailer_t>();
        if let Some(additional) = total_capacity.checked_sub(self.buffer.len()) {
            self.buffer.reserve(additional);
        }
    }

    #[inline]
    pub fn extend_inline_data(&mut self, data: &[u8]) {
        // Ensure we maintain space for the trailer
        let final_inline_len = self.inline_data().len() + data.len();
        if final_inline_len > self.capacity_inline {
            self.capacity_inline = final_inline_len;
            self.update_reservation();
        }
        unsafe {
            debug_assert!(self.buffer.capacity() - self.buffer.len() >= data.len());
            ptr::copy_nonoverlapping(data.as_ptr(), self.buffer.as_mut_ptr().offset(self.buffer.len() as isize), data.len());
            self.header_mut().msgh_size += data.len() as sys::mach_msg_size_t;
            self.buffer.set_len(self.buffer.len() + data.len());
        }
    }

    /// Attaches a port to a message, marking for the designated right to be copied on transmission.
    /// 
    /// It is the responsibility of the caller to ensure that the port lives until the message is sent or the port is removed
    /// from the message.
    pub unsafe fn copy_right(&mut self, mode: PortCopyMode, port: &Port) {
        self.copy_right_raw(mode, port.as_raw_port())
    }

    /// Attaches a port to a message, marking for the designated right to be copied on transmission.
    /// 
    /// It is the responsibility of the caller to ensure that the port lives until the message is sent or the port is removed
    /// from the message.
    pub unsafe fn copy_right_raw(&mut self, mode: PortCopyMode, port: RawPort) {
        let mut descriptor = sys::mach_msg_port_descriptor_t {
            name: port,
            pad1: 0,
            _bitfield_1: mem::zeroed(),
        };
        descriptor.set_type(sys::MACH_MSG_PORT_DESCRIPTOR);
        descriptor.set_disposition(match mode {
            PortCopyMode::Send => sys::MACH_MSG_TYPE_COPY_SEND,
            PortCopyMode::MakeSend => sys::MACH_MSG_TYPE_MAKE_SEND,
            PortCopyMode::MakeSendOnce => sys::MACH_MSG_TYPE_MAKE_SEND_ONCE,
        });
        self.append_descriptor(descriptor);
    }

    /// Attaches a port to a message, marking for the designated right to be moved on transmission.
    pub fn move_right(&mut self, mode: PortMoveMode, port: Port) {
        unsafe { self.move_right_raw(mode, port.into_raw_port()) }
    }

    /// Attaches a port to a message, marking for the designated right to be moved on transmission.
    pub unsafe fn move_right_raw(&mut self, mode: PortMoveMode, port: RawPort) {
        let mut descriptor = sys::mach_msg_port_descriptor_t {
            name: port,
            pad1: 0,
            _bitfield_1: mem::zeroed(),
        };
        descriptor.set_type(sys::MACH_MSG_PORT_DESCRIPTOR);
        descriptor.set_disposition(match mode {
            PortMoveMode::Receive => sys::MACH_MSG_TYPE_MOVE_RECEIVE,
            PortMoveMode::Send => sys::MACH_MSG_TYPE_MOVE_SEND,
            PortMoveMode::SendOnce => sys::MACH_MSG_TYPE_MOVE_SEND_ONCE,
        });
        self.append_descriptor(descriptor);
    }

    unsafe fn append_descriptor<T>(&mut self, descriptor: T) {
        // TODO: special case when there is no inline data to be shuffled?
        debug_assert!(mem::size_of::<T>() <= mem::size_of::<sys::mach_msg_descriptor_t>());
        let descriptor_bytes = slice::from_raw_parts(&descriptor as *const T as *const u8, mem::size_of::<T>());
        let insertion_offset = mem::size_of::<MessageStart>() + self.descriptors_byte_len();
        self.buffer.splice(insertion_offset..insertion_offset, descriptor_bytes.iter().cloned());
        *self.descriptor_count_mut() += 1;
        self.header_mut().msgh_bits |= sys::MACH_MSGH_BITS_COMPLEX;
        self.header_mut().msgh_size += mem::size_of::<T>() as sys::mach_msg_size_t;
        // Update reservations
        if self.descriptor_count() as usize > self.capacity_descriptors {
            self.capacity_descriptors = self.descriptor_count() as usize;
            self.update_reservation();
        }
    }
}

impl Msg {
    #[inline]
    pub fn inline_data(&self) -> &[u8] {
        debug_assert!(self.0.len() >= self.header().msgh_size as usize);
        let offset = mem::size_of::<MessageStart>() + self.descriptors_byte_len();
        unsafe { slice::from_raw_parts(self.0.as_ptr().offset(offset as isize), self.header().msgh_size as usize - offset) }
    }

    #[inline]
    pub fn inline_data_mut(&mut self) -> &mut [u8] {
        debug_assert!(self.0.len() >= self.header().msgh_size as usize);
        let offset = mem::size_of::<MessageStart>() + self.descriptors_byte_len();
        unsafe { slice::from_raw_parts_mut(self.0.as_mut_ptr().offset(offset as isize), self.header().msgh_size as usize - offset) }
    }

    #[inline]
    pub fn descriptors(&self) -> MsgDescriptorIter {
        MsgDescriptorIter {
           rem_count: self.descriptor_count(),
           ptr: unsafe { self.0.as_ptr().add(mem::size_of::<MessageStart>()) as *const MsgDescriptor },
           msg: PhantomData,
        }
    }

    #[inline]
    pub fn descriptors_mut(&mut self) -> MsgDescriptorIterMut {
        MsgDescriptorIterMut {
           rem_count: self.descriptor_count(),
           ptr: unsafe { self.0.as_mut_ptr().add(mem::size_of::<MessageStart>()) as *mut MsgDescriptor },
           msg: PhantomData,
        }
    }

    #[inline]
    pub fn descriptor_count(&self) -> usize {
        unsafe { (*(self.0.as_ptr() as *const MessageStart)).body.msgh_descriptor_count as usize }
    }

    #[inline]
    unsafe fn descriptor_count_mut(&mut self) -> &mut sys::mach_msg_size_t {
        &mut (*(self.0.as_mut_ptr() as *mut MessageStart)).body.msgh_descriptor_count
    }

    #[inline]
    fn descriptors_byte_len(&self) -> usize {
        let mut iter = self.descriptors();
        let start_ptr = iter.ptr;
        while let Some(_) = iter.next() {
        }
        iter.ptr as usize - start_ptr as usize
    }

    #[inline]
    pub fn complex(&self) -> bool {
        self.header().msgh_bits & sys::MACH_MSGH_BITS_COMPLEX != 0
    }

    #[inline]
    pub(crate) fn header(&self) -> &sys::mach_msg_header_t {
        debug_assert!(self.0.len() >= mem::size_of::<sys::mach_msg_header_t>());
        unsafe { &*(self.0.as_ptr() as *const sys::mach_msg_header_t) }
    }

    #[inline]
    pub(crate) fn header_mut(&mut self) -> &mut sys::mach_msg_header_t {
        debug_assert!(self.0.len() >= mem::size_of::<sys::mach_msg_header_t>());
        unsafe { &mut *(self.0.as_mut_ptr() as *mut sys::mach_msg_header_t) }
    }
}

impl MsgDescriptor {
    #[inline]
    pub fn kind(&self) -> MsgDescriptorKind {
        match self.0.type_() {
            sys::MACH_MSG_PORT_DESCRIPTOR => MsgDescriptorKind::Port(unsafe { &*(self as *const _ as *const MsgPortDescriptor) }),
            sys::MACH_MSG_OOL_DESCRIPTOR => MsgDescriptorKind::Ool(unsafe { &*(self as *const _ as *const MsgDescriptor) }),
            sys::MACH_MSG_OOL_PORTS_DESCRIPTOR => MsgDescriptorKind::OolPorts(unsafe { &*(self as *const _ as *const MsgDescriptor) }),
            sys::MACH_MSG_OOL_VOLATILE_DESCRIPTOR => MsgDescriptorKind::OolVolatile(unsafe { &*(self as *const _ as *const MsgDescriptor) }),
            _ => unreachable!(), 
        }
    }

    #[inline]
    pub fn kind_mut(&mut self) -> MsgDescriptorKindMut {
        match self.0.type_() {
            sys::MACH_MSG_PORT_DESCRIPTOR => MsgDescriptorKindMut::Port(unsafe { &mut *(self as *mut _ as *mut MsgPortDescriptor) }),
            sys::MACH_MSG_OOL_DESCRIPTOR => MsgDescriptorKindMut::Ool(unsafe { &mut *(self as *mut _ as *mut MsgDescriptor) }),
            sys::MACH_MSG_OOL_PORTS_DESCRIPTOR => MsgDescriptorKindMut::OolPorts(unsafe { &mut *(self as *mut _ as *mut MsgDescriptor) }),
            sys::MACH_MSG_OOL_VOLATILE_DESCRIPTOR => MsgDescriptorKindMut::OolVolatile(unsafe { &mut *(self as *mut _ as *mut MsgDescriptor) }),
            _ => unreachable!(), 
        }
    }

    #[inline]
    fn size(&self) -> usize {
        match self.0.type_() {
            sys::MACH_MSG_PORT_DESCRIPTOR => mem::size_of::<sys::mach_msg_port_descriptor_t>(),
            sys::MACH_MSG_OOL_DESCRIPTOR => mem::size_of::<sys::mach_msg_ool_descriptor_t>(),
            sys::MACH_MSG_OOL_PORTS_DESCRIPTOR => mem::size_of::<sys::mach_msg_ool_ports_descriptor_t>(),
            sys::MACH_MSG_OOL_VOLATILE_DESCRIPTOR => mem::size_of::<sys::mach_msg_ool_descriptor_t>(),
            _ => unreachable!(),
        }
    }

}

impl MsgPortDescriptor {
    #[inline]
    pub fn take_port(&mut self) -> io::Result<Option<Port>> {
        if let Some(port) = self.take_raw_port() {
            Ok(Some(unsafe { Port::from_raw_port(port)? }))
        } else {
            Ok(None)
        }
    }

    #[inline]
    pub fn take_raw_port(&mut self) -> Option<RawPort> {
        if self.0.name == sys::MACH_PORT_NULL || self.0.name == sys::MACH_PORT_DEAD {
            return None;
        }
        Some(mem::replace(&mut self.0.name, sys::MACH_PORT_NULL))
    }
}

impl Deref for MsgPortDescriptor {
    type Target = MsgDescriptor;

    #[inline]
    fn deref(&self) -> &MsgDescriptor {
        unsafe { &* { self as *const _ as *const MsgDescriptor } }
    }
}

impl fmt::Debug for MsgPortDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("MsgPortDescriptor")
            .field("name", &format_args!("{:#x?}", self.0.name))
            .field("disposition", &format_args!("{:?}", self.0.disposition()))
            .finish()
    }
}

pub struct MsgDescriptorIter<'a> {
    msg: PhantomData<&'a Msg>,
    ptr: *const MsgDescriptor,
    rem_count: usize,
}

impl<'a> Iterator for MsgDescriptorIter<'a> {
    type Item = &'a MsgDescriptor;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(new_count) = self.rem_count.checked_sub(1) {
            self.rem_count = new_count;
            unsafe {
                let current = &*self.ptr;
                self.ptr = (self.ptr as *const u8).add(current.size()) as *const MsgDescriptor;
                Some(current)
            }
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.rem_count, Some(self.rem_count))
    }

    #[inline]
    fn count(self) -> usize {
        self.rem_count
    }
}

impl<'a> ExactSizeIterator for MsgDescriptorIter<'a> {
}

pub struct MsgDescriptorIterMut<'a> {
    msg: PhantomData<&'a Msg>,
    ptr: *mut MsgDescriptor,
    rem_count: usize,
}

impl<'a> Iterator for MsgDescriptorIterMut<'a> {
    type Item = &'a mut MsgDescriptor;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(new_count) = self.rem_count.checked_sub(1) {
            self.rem_count = new_count;
            unsafe {
                let current = &mut *self.ptr;
                self.ptr = (self.ptr as *mut u8).add(current.size()) as *mut MsgDescriptor;
                Some(current)
            }
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.rem_count, Some(self.rem_count))
    }

    #[inline]
    fn count(self) -> usize {
        self.rem_count
    }
}

impl<'a> ExactSizeIterator for MsgDescriptorIterMut<'a> {
}

impl MsgImpl for MsgBuffer {
    fn as_ptr(&self) -> *const u8 {
        self.buffer.as_ptr()
    }
    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.buffer.as_mut_ptr()
    }

    fn len(&self) -> usize {
        self.buffer.len()
    }
    fn capacity(&self) -> usize {
        self.buffer.capacity()
    }
    unsafe fn set_len(&mut self, len: usize) {
        self.buffer.set_len(len)
    }

    fn reset_on_send(&mut self) {
        debug_assert!(self.buffer.len() >= mem::size_of::<MessageStart>());
        unsafe {
            self.buffer.set_len(mem::size_of::<MessageStart>());
            *(self.buffer.as_mut_ptr() as *mut MessageStart) = MessageStart {
                header: sys::mach_msg_header_t {
                    msgh_bits: sys::MACH_MSG_TYPE_COPY_SEND,
                    msgh_size: mem::size_of::<MessageStart>() as _,
                    msgh_remote_port: sys::MACH_PORT_NULL,
                    msgh_local_port: sys::MACH_PORT_NULL,
                    msgh_voucher_port: sys::MACH_PORT_NULL,
                    msgh_id: 0,
                },
                body: sys::mach_msg_body_t {
                    msgh_descriptor_count: 0,
                },
            };
            // FIXME: keep resources marked as copied?
        }
    }
}

impl fmt::Debug for Msg {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Msg {{ ")?;

        write!(f, "header: {{ ")?;
        write!(f, "complex: {:?}, ", self.complex())?;
        write!(f, "size: {:?} ", self.header().msgh_size)?;
        write!(f, "}} ")?;

        write!(f, "inline_data: {:?}", self.inline_data())?;

        write!(f, "}}")?;

        Ok(())
    }
}

impl fmt::Debug for MsgBuffer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (&**self).fmt(f)
    }
}

impl Deref for MsgBuffer {
    type Target = Msg;

    fn deref(&self) -> &Msg {
        let gen: &MsgImpl = self;
        unsafe { mem::transmute(gen) }
    }
}

impl DerefMut for MsgBuffer {
    fn deref_mut(&mut self) -> &mut Msg {
        let gen: &mut MsgImpl = self;
        unsafe { mem::transmute(gen) }
    }
}