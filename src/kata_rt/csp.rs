use tokio::sync::mpsc;
use tokio::sync::broadcast;
use tokio::task;

pub struct RendezvousChannel {
    pub tx: mpsc::Sender<*mut u8>,
    pub rx: mpsc::Receiver<*mut u8>,
}

pub struct QueueChannel {
    pub tx: mpsc::Sender<*mut u8>,
    pub rx: mpsc::Receiver<*mut u8>,
}

pub struct BroadcastChannel {
    pub tx: broadcast::Sender<*mut u8>,
}

impl RendezvousChannel {
    pub fn new() -> (mpsc::Sender<*mut u8>, mpsc::Receiver<*mut u8>) {
        mpsc::channel(1) // Emulating rendezvous with a buffer of 1, but we might want a pure rendezvous
    }
}

impl QueueChannel {
    pub fn new(size: usize) -> (mpsc::Sender<*mut u8>, mpsc::Receiver<*mut u8>) {
        mpsc::channel(size)
    }
}

impl BroadcastChannel {
    pub fn new(size: usize) -> (broadcast::Sender<*mut u8>, broadcast::Receiver<*mut u8>) {
        broadcast::channel(size)
    }
}
