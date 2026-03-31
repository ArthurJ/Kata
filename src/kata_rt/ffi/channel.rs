use crate::csp::{RendezvousChannel, QueueChannel, BroadcastChannel};

// C-ABI para retornar dois handles (sender e receiver)
#[repr(C)]
pub struct ChannelPair {
    pub sender: *mut u8,
    pub receiver: *mut u8,
}

#[no_mangle]
pub extern "C" fn kata_rt_chan_create_rendezvous() -> ChannelPair {
    let (tx, rx) = RendezvousChannel::new();
    let sender = Box::into_raw(Box::new(tx)) as *mut u8;
    let receiver = Box::into_raw(Box::new(rx)) as *mut u8;
    ChannelPair { sender, receiver }
}

#[no_mangle]
pub extern "C" fn kata_rt_chan_create_queue(size: usize) -> ChannelPair {
    let (tx, rx) = QueueChannel::new(size);
    let sender = Box::into_raw(Box::new(tx)) as *mut u8;
    let receiver = Box::into_raw(Box::new(rx)) as *mut u8;
    ChannelPair { sender, receiver }
}

#[no_mangle]
pub extern "C" fn kata_rt_chan_create_broadcast(size: usize) -> *mut BroadcastChannel {
    let (tx, _rx) = BroadcastChannel::new(size);
    let chan = Box::new(BroadcastChannel { tx });
    Box::into_raw(chan)
}