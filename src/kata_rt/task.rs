use super::memory::LocalArena;
use std::cell::RefCell;
use wasmtime_fiber::{Fiber, FiberStack, Suspend};
use crate::kata_rt::csp::{KataSender, KataReceiver};

pub enum FiberIntent {
    None,
    Yield,
    SendRendezvous(*mut KataSender, *mut u8),
    RecvRendezvous(*mut KataReceiver),
    SendQueue(*mut KataSender, *mut u8),
    RecvQueue(*mut KataReceiver),
    SendBroadcast(*mut KataSender, *mut u8),
    RecvBroadcast(*mut KataReceiver),
}

thread_local! {
    pub static LOCAL_ARENA: RefCell<LocalArena> = RefCell::new(LocalArena::new());
    // O ponteiro da suspensao agora tipado para aceitar FiberIntent como saida (Yield) e retornar *mut u8 (Resume)
    pub static CURRENT_SUSPEND: RefCell<Option<*mut Suspend<*mut u8, FiberIntent, ()>>> = RefCell::new(None);
}

pub fn alloc_local(size: usize, align: usize) -> *mut u8 {
    LOCAL_ARENA.with(|arena| {
        arena.borrow_mut().alloc(size, align)
    })
}

pub fn clear_local() {
    LOCAL_ARENA.with(|arena| {
        arena.borrow_mut().clear();
    })
}

/// Helper para pausar a execucao da Action C-ABI sincrona e enviar uma intencao ao Tokio.
pub fn yield_cooperative(intent: FiberIntent) -> *mut u8 {
    CURRENT_SUSPEND.with(|suspend_ref| {
        if let Some(suspend_ptr) = *suspend_ref.borrow() {
            unsafe {
                (*suspend_ptr).suspend(intent)
            }
        } else {
            std::ptr::null_mut()
        }
    })
}

/// Cria e roda uma Fiber usando a function pointer passada pelo Cranelift
pub async fn spawn_fiber_action(action: extern "C" fn()) {
    let stack = FiberStack::new(256 * 1024, false).expect("Falha ao alocar a Pilha da Corrotina (FiberStack)");

    let mut fiber = Fiber::new(stack, move |_first_arg: *mut u8, suspend: &mut Suspend<*mut u8, FiberIntent, ()>| {
        let suspend_ptr: *mut Suspend<*mut u8, FiberIntent, ()> = suspend as *mut _;
        
        CURRENT_SUSPEND.with(|s| {
            *s.borrow_mut() = Some(suspend_ptr);
        });

        action();

        CURRENT_SUSPEND.with(|s| {
            *s.borrow_mut() = None;
        });
        clear_local();

    }).expect("Falha ao criar o Fiber");

    let mut resume_val = std::ptr::null_mut();

    loop {
        match fiber.resume(resume_val) {
            Ok(()) => {
                // Fim da execucao
                break;
            }
            Err(intent) => {
                match intent {
                    FiberIntent::Yield => {
                        tokio::task::yield_now().await;
                        resume_val = std::ptr::null_mut();
                    }
                    FiberIntent::SendRendezvous(sender_ptr, val) => {
                        if !sender_ptr.is_null() {
                            let sender = unsafe { &mut *sender_ptr };
                            if let KataSender::Rendezvous(tx) = sender {
                                let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
                                if tx.send((val, Some(ack_tx))).await.is_ok() {
                                    let _ = ack_rx.await;
                                }
                            }
                        }
                        resume_val = std::ptr::null_mut();
                    }
                    FiberIntent::RecvRendezvous(recv_ptr) => {
                        if !recv_ptr.is_null() {
                            let receiver = unsafe { &mut *recv_ptr };
                            if let KataReceiver::Rendezvous(rx) = receiver {
                                if let Some((val, ack_tx)) = rx.recv().await {
                                    if let Some(ack) = ack_tx {
                                        let _ = ack.send(());
                                    }
                                    resume_val = val;
                                    continue;
                                }
                            }
                        }
                        resume_val = std::ptr::null_mut();
                    }
                    FiberIntent::SendQueue(sender_ptr, val) => {
                        if !sender_ptr.is_null() {
                            let sender = unsafe { &mut *sender_ptr };
                            if let KataSender::Queue(tx) = sender {
                                let _ = tx.send(val).await;
                            }
                        }
                        resume_val = std::ptr::null_mut();
                    }
                    FiberIntent::RecvQueue(recv_ptr) => {
                        if !recv_ptr.is_null() {
                            let receiver = unsafe { &mut *recv_ptr };
                            if let KataReceiver::Queue(rx) = receiver {
                                if let Some(val) = rx.recv().await {
                                    resume_val = val;
                                    continue;
                                }
                            }
                        }
                        resume_val = std::ptr::null_mut();
                    }
                    FiberIntent::SendBroadcast(sender_ptr, val) => {
                        if !sender_ptr.is_null() {
                            let sender = unsafe { &mut *sender_ptr };
                            if let KataSender::Broadcast(tx) = sender {
                                let _ = tx.send(val);
                            }
                        }
                        resume_val = std::ptr::null_mut();
                    }
                    FiberIntent::RecvBroadcast(recv_ptr) => {
                        if !recv_ptr.is_null() {
                            let receiver = unsafe { &mut *recv_ptr };
                            if let KataReceiver::Broadcast(rx) = receiver {
                                if let Ok(val) = rx.recv().await {
                                    resume_val = val;
                                    continue;
                                }
                            }
                        }
                        resume_val = std::ptr::null_mut();
                    }
                    FiberIntent::None => {
                        resume_val = std::ptr::null_mut();
                    }
                }
            }
        }
    }
}
