pub mod memory;
pub mod csp;
pub mod task;
pub mod ffi;
#[cfg(test)]
pub mod tests;

use tokio::runtime::Runtime;

#[no_mangle]
pub extern "C" fn kata_rt_boot(main_action: extern "C" fn()) {
    log::info!("Iniciando Kata Runtime (Tokio M:N Scheduler)");

    let rt = Runtime::new().expect("Falha ao inicializar o tokio::Runtime");

    rt.block_on(async {
        // Here we could spawn the main_action if it was compiled as a Future.
        // For now, since it's just a C function pointer, we call it in a blocking task
        // or directly if it's supposed to start the state machine itself.
        // Assuming main_action is synchronous for the bootstrap:
        tokio::task::spawn_blocking(move || {
            main_action();
        }).await.unwrap();
    });

    log::info!("Kata Runtime finalizado.");
}

#[allow(dead_code)]
pub fn init_stub() {
    log::info!("Iniciando Kata Runtime Embeddado (Stub)");
}
