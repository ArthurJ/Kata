use crate::kata_rt::csp::{RendezvousChannel, QueueChannel, BroadcastChannel};

// C-ABI doesn't cleanly support returning complex tuples, so we return a boxed struct pointer
// that the Cranelift code will unpack or use as an opaque handle.

#[no_mangle]
pub extern "C" fn kata_rt_chan_create_rendezvous() -> *mut RendezvousChannel {
    let (tx, rx) = RendezvousChannel::new();
    let chan = Box::new(RendezvousChannel { tx, rx });
    Box::into_raw(chan)
}

#[no_mangle]
pub extern "C" fn kata_rt_chan_create_queue(size: usize) -> *mut QueueChannel {
    let (tx, rx) = QueueChannel::new(size);
    let chan = Box::new(QueueChannel { tx, rx });
    Box::into_raw(chan)
}

#[no_mangle]
pub extern "C" fn kata_rt_chan_create_broadcast(size: usize) -> *mut BroadcastChannel {
    let (tx, _rx) = BroadcastChannel::new(size);
    let chan = Box::new(BroadcastChannel { tx });
    Box::into_raw(chan)
}
