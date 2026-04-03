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
        // Usa a nova infraestrutura de corrotinas stackful em vez de spawn_blocking pesado!
        crate::kata_rt::task::spawn_fiber_action(main_action).await;
    });

    log::info!("Kata Runtime finalizado.");
}
