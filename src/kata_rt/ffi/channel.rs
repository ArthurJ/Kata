
use crate::kata_rt::csp::{KataSender, KataReceiver, BroadcastChannel};
use crate::kata_rt::task::alloc_local;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

#[no_mangle]
pub extern "C" fn kata_rt_chan_create_rendezvous() -> *mut u8 {
    let (tx, rx) = KataSender::new_rendezvous();
    let tuple_ptr = alloc_local(16, 8) as *mut *mut u8;
    unsafe {
        *tuple_ptr = Box::into_raw(Box::new(tx)) as *mut u8;
        *tuple_ptr.add(1) = Box::into_raw(Box::new(rx)) as *mut u8;
    }
    tuple_ptr as *mut u8
}

#[no_mangle]
pub extern "C" fn kata_rt_chan_create_queue(size: usize) -> *mut u8 {
    let (tx, rx) = KataSender::new_queue(size);
    let tuple_ptr = alloc_local(16, 8) as *mut *mut u8;
    unsafe {
        *tuple_ptr = Box::into_raw(Box::new(tx)) as *mut u8;
        *tuple_ptr.add(1) = Box::into_raw(Box::new(rx)) as *mut u8;
    }
    tuple_ptr as *mut u8
}

#[no_mangle]
pub extern "C" fn kata_rt_chan_create_broadcast() -> *mut u8 {
    let (tx, _rx) = tokio::sync::broadcast::channel(16); // Default broadcast size
    let b_tx = KataSender::Broadcast(tx.clone());
    let b_chan = BroadcastChannel { tx };
    
    let tuple_ptr = alloc_local(16, 8) as *mut *mut u8;
    unsafe {
        *tuple_ptr = Box::into_raw(Box::new(b_tx)) as *mut u8;
        *tuple_ptr.add(1) = Box::into_raw(Box::new(b_chan)) as *mut u8; // We treat the subscribe as returning a special channel pointer
    }
    tuple_ptr as *mut u8
}

#[no_mangle]
pub extern "C" fn kata_rt_chan_send(sender_ptr: *mut u8, val: *mut u8) {
    if sender_ptr.is_null() { return; }
    
    let sender = unsafe { &*(sender_ptr as *mut KataSender) };
    let intent = match sender {
        KataSender::Rendezvous(_) => crate::kata_rt::task::FiberIntent::SendRendezvous(sender_ptr as *mut KataSender, val),
        KataSender::Queue(_) => crate::kata_rt::task::FiberIntent::SendQueue(sender_ptr as *mut KataSender, val),
        KataSender::Broadcast(_) => crate::kata_rt::task::FiberIntent::SendBroadcast(sender_ptr as *mut KataSender, val),
    };
    
    crate::kata_rt::task::yield_cooperative(intent);
}

#[no_mangle]
pub extern "C" fn kata_rt_chan_recv(recv_ptr: *mut u8) -> *mut u8 {
    if recv_ptr.is_null() { return std::ptr::null_mut(); }
    
    let receiver = unsafe { &*(recv_ptr as *mut KataReceiver) };
    let intent = match receiver {
        KataReceiver::Rendezvous(_) => crate::kata_rt::task::FiberIntent::RecvRendezvous(recv_ptr as *mut KataReceiver),
        KataReceiver::Queue(_) => crate::kata_rt::task::FiberIntent::RecvQueue(recv_ptr as *mut KataReceiver),
        KataReceiver::Broadcast(_) => crate::kata_rt::task::FiberIntent::RecvBroadcast(recv_ptr as *mut KataReceiver),
    };
    
    crate::kata_rt::task::yield_cooperative(intent)
}
