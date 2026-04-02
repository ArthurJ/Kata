
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
    let sender = unsafe { &mut *(sender_ptr as *mut KataSender) };
    
    // We are inside a spawn_blocking! So we can use Handle::current().block_on()
    if let Ok(handle) = Handle::try_current() {
        handle.block_on(async {
            match sender {
                KataSender::Rendezvous(tx) => {
                    let (ack_tx, ack_rx) = oneshot::channel();
                    if tx.send((val, Some(ack_tx))).await.is_ok() {
                        let _ = ack_rx.await; // Wait for receiver to process it!
                    }
                }
                KataSender::Queue(tx) => {
                    let _ = tx.send(val).await;
                }
                KataSender::Broadcast(tx) => {
                    let _ = tx.send(val);
                }
            }
        });
    }
}

#[no_mangle]
pub extern "C" fn kata_rt_chan_recv(recv_ptr: *mut u8) -> *mut u8 {
    if recv_ptr.is_null() { return std::ptr::null_mut(); }
    let receiver = unsafe { &mut *(recv_ptr as *mut KataReceiver) };
    
    if let Ok(handle) = Handle::try_current() {
        handle.block_on(async {
            match receiver {
                KataReceiver::Rendezvous(rx) => {
                    if let Some((val, ack_tx)) = rx.recv().await {
                        if let Some(ack) = ack_tx {
                            let _ = ack.send(()); // Acknowledge receipt
                        }
                        return val;
                    }
                }
                KataReceiver::Queue(rx) => {
                    if let Some(val) = rx.recv().await {
                        return val;
                    }
                }
                KataReceiver::Broadcast(rx) => {
                    if let Ok(val) = rx.recv().await {
                        return val;
                    }
                }
            }
            std::ptr::null_mut()
        })
    } else {
        std::ptr::null_mut()
    }
}
