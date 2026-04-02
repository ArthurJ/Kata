
use tokio::sync::{mpsc, broadcast, oneshot};

pub enum KataSender {
    Rendezvous(mpsc::Sender<(*mut u8, Option<oneshot::Sender<()>>)>),
    Queue(mpsc::Sender<*mut u8>),
    Broadcast(broadcast::Sender<*mut u8>),
}

pub enum KataReceiver {
    Rendezvous(mpsc::Receiver<(*mut u8, Option<oneshot::Sender<()>>)>),
    Queue(mpsc::Receiver<*mut u8>),
    Broadcast(broadcast::Receiver<*mut u8>),
}

// We also need a Subscribe mechanism for broadcast.
// Kata broadcast returns (Sender, SubscribeFunc).
// Since C-ABI is tricky, we can just return a special Channel that can be subscribed to.
pub struct BroadcastChannel {
    pub tx: broadcast::Sender<*mut u8>,
}

impl KataSender {
    pub fn new_rendezvous() -> (Self, KataReceiver) {
        let (tx, rx) = mpsc::channel(1);
        (KataSender::Rendezvous(tx), KataReceiver::Rendezvous(rx))
    }
    
    pub fn new_queue(size: usize) -> (Self, KataReceiver) {
        let (tx, rx) = mpsc::channel(size);
        (KataSender::Queue(tx), KataReceiver::Queue(rx))
    }
}
