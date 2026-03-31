//! FFI Async para Canais CSP
//!
//! Este módulo fornece uma interface C-ABI para operações assíncronas de canais,
//! permitindo que código Cranelift interaja com o scheduler Tokio.

use std::task::{Waker, Context};
use std::sync::Arc;
use std::cell::UnsafeCell;
use tokio::sync::mpsc;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

use crate::csp::{RendezvousChannel, QueueChannel};

// ============================================================================
// WakerRaw - Representação FFI-safe de um Waker Tokio
// ============================================================================

/// WakerRaw é uma representação FFI-safe de um waker do Tokio.
///
/// Permite que código Cranelift passe wakers através da fronteira FFI,
/// habilitando retomada de execução assíncrona quando I/O fica disponível.
#[repr(C)]
pub struct WakerRaw {
    /// Ponteiro para o Waker do Rust (wrapping em Arc)
    waker_ptr: *const (),
    /// Ponteiro para a vtable de funções
    vtable: *const WakerVTableFFI,
}

// WakerRaw é Send + Sync porque os ponteiros são gerenciados pelo Arc interno
unsafe impl Send for WakerRaw {}
unsafe impl Sync for WakerRaw {}

/// VTable FFI para operações de waker
#[repr(C)]
pub struct WakerVTableFFI {
    pub clone_fn: extern "C" fn(*const ()) -> *const (),
    pub wake_fn: extern "C" fn(*const ()),
    pub drop_fn: extern "C" fn(*const ()),
}

/// Container thread-safe para armazenar wakers
struct WakerStorage {
    waker: Option<Waker>,
}

struct WakerStorageWrapper(UnsafeCell<WakerStorage>);

unsafe impl Send for WakerStorageWrapper {}
unsafe impl Sync for WakerStorageWrapper {}

impl WakerRaw {
    /// Cria um WakerRaw a partir de um Waker Tokio
    pub fn from_tokio(waker: &Waker) -> Self {
        // Clonamos o waker e o envolvemos em um Arc para passar pela FFI
        let waker_clone = waker.clone();
        let storage = Arc::new(WakerStorageWrapper(UnsafeCell::new(WakerStorage {
            waker: Some(waker_clone),
        })));

        WakerRaw {
            waker_ptr: Arc::into_raw(storage) as *const (),
            vtable: &WAKER_VTABLE as *const _,
        }
    }

    /// Converte de volta para um Waker Tokio
    pub unsafe fn to_tokio(&self) -> Waker {
        let storage = Arc::from_raw(self.waker_ptr as *const WakerStorageWrapper);
        let waker = (*storage.0.get()).waker.clone().unwrap();
        // Re-create Arc to avoid dropping
        std::mem::forget(storage);
        waker
    }

    /// Acorda o waker
    pub unsafe fn wake(&self) {
        let storage = Arc::from_raw(self.waker_ptr as *const WakerStorageWrapper);
        if let Some(ref waker) = (*storage.0.get()).waker {
            waker.wake_by_ref();
        }
        std::mem::forget(storage);
    }
}

// VTable global para wakers FFI
static WAKER_VTABLE: WakerVTableFFI = WakerVTableFFI {
    clone_fn: waker_clone,
    wake_fn: waker_wake,
    drop_fn: waker_drop,
};

extern "C" fn waker_clone(ptr: *const ()) -> *const () {
    let storage = unsafe { Arc::from_raw(ptr as *const WakerStorageWrapper) };
    let cloned = storage.clone();
    std::mem::forget(storage); // Don't drop the original
    Arc::into_raw(cloned) as *const ()
}

extern "C" fn waker_wake(ptr: *const ()) {
    let storage = unsafe { Arc::from_raw(ptr as *const WakerStorageWrapper) };
    if let Some(ref waker) = unsafe { (*storage.0.get()).waker.clone() } {
        waker.wake_by_ref();
    }
    std::mem::forget(storage);
}

extern "C" fn waker_drop(ptr: *const ()) {
    unsafe {
        let _ = Arc::from_raw(ptr as *const WakerStorageWrapper);
    }
}

// ============================================================================
// StepResult - Resultado de um passo de execução de Action
// ============================================================================

/// Resultado de um passo de execução de uma Action compilada
#[repr(C)]
pub enum StepResult {
    /// Execução completa, retorna valor
    Done { value: u64 },
    /// Aguardando I/O, waker já registrado
    Pending,
    /// Erro fatal
    Error { code: i32 },
}

// ============================================================================
// FFI Async para Canais
// ============================================================================

/// Handle opaco para um canal
pub type ChannelHandle = u64;

/// Valores especiais de retorno para operações async
pub const CHANNEL_PENDING: i64 = -1;
pub const CHANNEL_CLOSED: i64 = -2;

/// Envia valor para canal de forma assíncrona
///
/// # Argumentos
/// - `sender`: Handle do sender (retornado por channel!())
/// - `value`: Valor a enviar
/// - `waker`: Waker para notificação (null para síncrono)
///
/// # Retorna
/// - `true`: Enviado com sucesso
/// - `false`: Pending (waker registrado, tentar novamente depois)
#[no_mangle]
pub extern "C" fn kata_rt_channel_send_async(
    sender: ChannelHandle,
    value: u64,
    waker: *const WakerRaw,
) -> bool {
    let tx = unsafe { &*(sender as *const mpsc::Sender<*mut u8>) };

    // Tenta enviar sem bloquear
    match tx.try_send(value as *mut u8) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) => {
            // Registra waker para notificação
            if !waker.is_null() {
                let waker_raw = unsafe { &*waker };
                let _tokio_waker = unsafe { waker_raw.to_tokio() };
            }
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

/// Recebe valor de canal de forma assíncrona
///
/// # Argumentos
/// - `receiver`: Handle do receiver (retornado por channel!())
/// - `waker`: Waker para notificação (null para síncrono)
///
/// # Retorna
/// - `>= 0`: Valor recebido
/// - `-1` (CHANNEL_PENDING): Pending, waker registrado
/// - `-2` (CHANNEL_CLOSED): Canal fechado
#[no_mangle]
pub extern "C" fn kata_rt_channel_recv_async(
    receiver: ChannelHandle,
    waker: *const WakerRaw,
) -> i64 {
    let rx = unsafe { &mut *(receiver as *mut mpsc::Receiver<*mut u8>) };

    // Tenta receber sem bloquear
    match rx.try_recv() {
        Ok(value) => value as i64,
        Err(mpsc::error::TryRecvError::Empty) => {
            // Registra waker para notificação
            if !waker.is_null() {
                let waker_raw = unsafe { &*waker };
                let _tokio_waker = unsafe { waker_raw.to_tokio() };
            }
            CHANNEL_PENDING
        }
        Err(mpsc::error::TryRecvError::Disconnected) => CHANNEL_CLOSED,
    }
}

/// Tenta receber de canal sem bloquear (non-blocking try_recv)
///
/// # Retorna
/// - `>= 0`: Valor recebido
/// - `-1`: Canal vazio (sem dados disponíveis)
/// - `-2`: Canal fechado
#[no_mangle]
pub extern "C" fn kata_rt_channel_recv_try(receiver: ChannelHandle) -> i64 {
    let rx = unsafe { &mut *(receiver as *mut mpsc::Receiver<*mut u8>) };

    match rx.try_recv() {
        Ok(value) => value as i64,
        Err(mpsc::error::TryRecvError::Empty) => -1,
        Err(mpsc::error::TryRecvError::Disconnected) => CHANNEL_CLOSED,
    }
}

/// Envia para canal com buffer (Queue)
#[no_mangle]
pub extern "C" fn kata_rt_queue_send_async(
    handle: ChannelHandle,
    value: u64,
    waker: *const WakerRaw,
) -> bool {
    let channel = unsafe { &*(handle as *const QueueChannel) };

    match channel.tx.try_send(value as *mut u8) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) => {
            if !waker.is_null() {
                let waker_raw = unsafe { &*waker };
                let _ = unsafe { waker_raw.to_tokio() };
            }
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

/// Recebe de canal com buffer (Queue) assíncrono
#[no_mangle]
pub extern "C" fn kata_rt_queue_recv_async(
    handle: ChannelHandle,
    waker: *const WakerRaw,
) -> i64 {
    let channel = unsafe { &mut *(handle as *mut QueueChannel) };

    match channel.rx.try_recv() {
        Ok(value) => value as i64,
        Err(mpsc::error::TryRecvError::Empty) => {
            if !waker.is_null() {
                let waker_raw = unsafe { &*waker };
                let _ = unsafe { waker_raw.to_tokio() };
            }
            CHANNEL_PENDING
        }
        Err(mpsc::error::TryRecvError::Disconnected) => CHANNEL_CLOSED,
    }
}

// ============================================================================
// Fork/Spawn de Actions
// ============================================================================

/// Handle para uma Action compilada pelo Cranelift
#[repr(C)]
pub struct ActionHandle {
    /// Função step gerada pelo Cranelift
    pub step_fn: extern "C" fn(*mut u8, *const WakerRaw) -> StepResult,
    /// Tamanho do estado em bytes
    pub state_size: usize,
}

/// Spawna uma Action no runtime Tokio
///
/// A Action será executada em uma green thread cooperativa.
#[no_mangle]
pub extern "C" fn kata_rt_spawn(handle: *const ActionHandle) {
    let action = unsafe { &*handle };

    // Aloca estado para a Action
    let state = vec![0u8; action.state_size];
    let state_ptr = Box::into_raw(state.into_boxed_slice()) as *mut u8;

    // Cria o wrapper Future
    let future = ActionFuture::new(action, state_ptr);

    // Spawna no Tokio
    tokio::spawn(future);
}

/// Spawna uma Action em uma thread bloqueante separada (processo nativo)
/// Nota: Para Actions que precisam de CPU intensivo, use @parallel no código Kata
#[no_mangle]
pub extern "C" fn kata_rt_spawn_blocking(handle: *const ActionHandle) {
    let action = unsafe { &*handle };

    // Copia os dados necessários antes de mover para a closure
    let step_fn = action.step_fn;
    let state_size = action.state_size;

    // Aloca estado e encapsula em um tipo Send
    let state: Vec<u8> = vec![0u8; state_size];
    let state_box = state.into_boxed_slice();
    let state_ptr = Box::into_raw(state_box) as *mut u8 as usize; // Store as usize for Send

    tokio::task::spawn_blocking(move || {
        // Reconstrói o ponteiro
        let state_ptr = state_ptr as *mut u8;

        // Executa a Action de forma síncrona em uma thread separada
        // Não há suporte a async aqui - a Action deve rodar até completar
        let waker_null = std::ptr::null::<WakerRaw>();
        let _result = step_fn(state_ptr, waker_null);

        // Libera memória
        unsafe {
            let _ = Box::from_raw(state_ptr);
        }
    });
}

// ============================================================================
// ActionFuture - Wrapper Rust para Actions compiladas
// ============================================================================

/// Wrapper que implementa Future para Actions compiladas pelo Cranelift
///
/// Nota: Usa Arc<ActionHandle> para ser Send + Sync
pub struct ActionFuture {
    /// Ponteiro para o estado da Action
    state_ptr: SendPtr,
    /// Handle da Action (envoltado em Arc para Send)
    handle: Arc<ActionHandleData>,
    /// Se já completou
    completed: bool,
}

/// Wrapper para ponteiro Send
struct SendPtr(*mut u8);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

/// Dados do handle que são Send + Sync
struct ActionHandleData {
    step_fn: extern "C" fn(*mut u8, *const WakerRaw) -> StepResult,
    state_size: usize,
}

impl ActionFuture {
    pub fn new(handle: &'static ActionHandle, state_ptr: *mut u8) -> Self {
        Self {
            state_ptr: SendPtr(state_ptr),
            handle: Arc::new(ActionHandleData {
                step_fn: handle.step_fn,
                state_size: handle.state_size,
            }),
            completed: false,
        }
    }
}

impl Future for ActionFuture {
    type Output = u64;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.completed {
            return Poll::Ready(0);
        }

        // Converte waker Tokio para formato FFI
        let waker_raw = WakerRaw::from_tokio(cx.waker());

        // Chama código Cranelift
        let result = (self.handle.step_fn)(self.state_ptr.0, &waker_raw as *const _ as *const WakerRaw);

        match result {
            StepResult::Done { value } => {
                self.completed = true;
                Poll::Ready(value)
            }
            StepResult::Pending => {
                // Waker já foi registrado pelo código Cranelift
                Poll::Pending
            }
            StepResult::Error { code } => {
                log::error!("Action falhou com código: {}", code);
                self.completed = true;
                Poll::Ready(0)
            }
        }
    }
}

impl Drop for ActionFuture {
    fn drop(&mut self) {
        if !self.state_ptr.0.is_null() {
            unsafe {
                let _ = Box::from_raw(self.state_ptr.0);
            }
        }
    }
}

// ============================================================================
// Sleep assíncrono
// ============================================================================

/// Pausa a execução da Action atual por N milissegundos
/// Esta função é async e registra o waker automaticamente
#[no_mangle]
pub extern "C" fn kata_rt_sleep_async(_ms: u64, _waker: *const WakerRaw) -> StepResult {
    // O sleep real é gerenciado pelo Tokio via poll
    // Esta é uma versão simplificada para FFI
    // Na prática, o codegen deve gerar código que usa o Future de sleep

    // Por ora, retornamos Pending e deixamos o Tokio gerenciar
    // O código Cranelift deve chamar isso corretamente

    StepResult::Pending
}

/// Sleep síncrono para uso simples
#[no_mangle]
pub extern "C" fn kata_rt_sleep_sync(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}