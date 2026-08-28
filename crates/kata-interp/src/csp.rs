//! CSP Nível 2 — fork!/spawn! via scheduler de fibers.
//!
//! O interpretador não tem codegen/Cranelift, então não tem `fn_ptr` (ponteiro
//! de função JIT-compilada). A solução é `interp_trampoline` — uma função
//! `extern "C" fn(i64, i64, i64, i64) -> i64` registrada como `fn_ptr` que
//! despacha de volta para o interpretador.
//!
//! Antes de `kata_rt_spawn`, o interpretador registra a action numa tabela
//! global e obtém `action_id`. O trampoline lê `action_id` do primeiro i64
//! de `args_ptr`, recupera a `(action_name, module)` da tabela, e executa
//! a action via `InterpCtx::new_with_arena`.

use std::sync::{Arc, Mutex, OnceLock};

use kata_inference::{TypedExprKind, TypedModule};

use crate::env::Env;
use crate::eval::{InterpCtx, InterpError, eval};

/// Entrada da tabela global de actions registradas para spawn.
#[derive(Clone)]
struct InterpActionEntry {
    action_name: String,
    module: Arc<TypedModule>,
    /// Tipos dos argumentos passados no fork!/spawn! — para despacho
    /// de overloads com mesma aridade mas assinaturas diferentes.
    arg_tys: Vec<kata_core::ty::Ty>,
    /// Enum registry para show de variants em fibers.
    enum_registry: Arc<kata_core::EnumRegistry>,
}

/// Tabela global de actions — indexada por `action_id` (índice no Vec).
///
/// O `action_id` é empacotado no `args_ptr` (primeiro i64) para que o
/// trampoline possa recuperar a action e o module.
static INTERP_ACTIONS: OnceLock<Mutex<Vec<InterpActionEntry>>> = OnceLock::new();

fn actions_table() -> &'static Mutex<Vec<InterpActionEntry>> {
    INTERP_ACTIONS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Registra uma action na tabela global e retorna o `action_id`.
///
/// Chamada antes de `kata_rt_spawn` para que o trampoline possa despachar
/// de volta para o interpretador.
pub(crate) fn register_action(
    action_name: &str,
    module: &Arc<TypedModule>,
    arg_tys: Vec<kata_core::ty::Ty>,
    enum_registry: Arc<kata_core::EnumRegistry>,
) -> i64 {
    let mut table = actions_table().lock().unwrap();
    let id = table.len() as i64;
    table.push(InterpActionEntry {
        action_name: action_name.to_string(),
        module: module.clone(),
        arg_tys,
        enum_registry,
    });
    id
}

/// Trampoline para `kata_rt_spawn` — despacha de volta para o interpretador.
///
/// Assinatura: `extern "C" fn(rt: i64, fiber_arena: i64, caller_arena: i64, args_ptr: i64) -> i64`
///
/// `args_ptr` aponta para uma struct `(action_id: i64, args_ptr_real: i64)`
/// alocada na arena pelo `eval` de `Fork`.
///
/// O trampoline:
/// 1. Lê `action_id` de `args_ptr` (primeiro i64)
/// 2. Lê `args_ptr_real` (segundo i64 — a tupla original)
/// 3. Recupera `(action_name, module)` da tabela global
/// 4. Cria `InterpCtx::new_with_arena(module, rt, fiber_arena)`
/// 5. Cria `Env` novo, desserializa args da tupla
/// 6. Encontra a action pelo nome no module
/// 7. Executa a action
/// 8. Retorna resultado (i64)
///
/// Dentro do fiber, FFIs de canal/sleep suspendem automaticamente via
/// `with_suspend` (TLS `CURRENT_SUSPEND` é setada pelo trampoline do fiber).
/// O interpretador não precisa fazer nada especial — as FFIs já fazem o
/// trabalho.
#[unsafe(no_mangle)]
pub extern "C" fn interp_trampoline(
    rt: i64,
    fiber_arena: i64,
    _caller_arena: i64,
    args_ptr: i64,
) -> i64 {
    // 1. Ler action_id e args_ptr_real da struct empacotada.
    let packed = args_ptr as *const i64;
    let action_id = unsafe { std::ptr::read(packed) };
    let args_ptr_real = unsafe { std::ptr::read(packed.add(1)) };

    // 2. Recuperar (action_name, module) da tabela global.
    let entry = {
        let table = actions_table().lock().unwrap();
        table.get(action_id as usize).cloned()
    };

    let Some(entry) = entry else {
        eprintln!("interp_trampoline: action_id {action_id} não encontrado");
        return 0;
    };

    // 3. Criar InterpCtx reusando a fiber_arena do scheduler.
    let mut ctx = InterpCtx::new_with_arena_registry(
        entry.module.clone(),
        rt,
        fiber_arena,
        entry.enum_registry.clone(),
    );

    // 4. Criar Env novo e desserializar args da tupla.
    let mut env = Env::new();

    // Definir stdio bindings — fibers forked precisam de __stdout__ etc.
    // Sem isto, echo! e @log{file: __stdout__} falham com "variável não definida".
    // Ver eval_entry_with_env em eval.rs para o mesmo padrão.
    env.define("__stdin__", kata_rt::kata_rt_stdin());
    env.define("__stdout__", kata_rt::kata_rt_stdout());
    env.define("__stderr__", kata_rt::kata_rt_stderr());

    let mut arg_vals: Vec<i64> = Vec::new();

    if args_ptr_real != 0 {
        // args_ptr_real aponta para uma tupla na arena — N i64s consecutivos.
        // Precisamos saber N. O TypedAction tem param_types, mas não temos
        // acesso direto aqui. Em vez disso, lemos n_args do terceiro i64
        // da struct empacotada (ver `eval` de Fork).
        let n_args = unsafe { std::ptr::read(packed.add(2)) };
        for i in 0..n_args as usize {
            let val = unsafe { std::ptr::read((args_ptr_real as *const i64).add(i)) };
            arg_vals.push(val);
        }
    }

    // 5. Encontrar a action pelo nome no module e executar.
    // Despachar por nome E aridade + compatibilidade de tipos.
    let module = ctx.module.clone();
    let n_args = arg_vals.len();
    let candidates: Vec<&kata_inference::TypedAction> = module
        .actions
        .iter()
        .filter(|a| a.name == entry.action_name && a.param_types.len() == n_args)
        .collect();

    let action = if candidates.is_empty() {
        eprintln!(
            "interp_trampoline: action '{}' não encontrada (aridade {})",
            entry.action_name, n_args
        );
        return 0;
    } else if candidates.len() == 1 {
        candidates[0]
    } else {
        // Despachar por tipo usando arg_tys da entrada
        candidates
            .iter()
            .max_by_key(|a| crate::eval::ty_compat_score(&entry.arg_tys, &a.param_types))
            .copied()
            .expect("candidates é não-vazio")
    };

    // 6. Bindar argumentos e executar o body da action.
    env.push_scope();
    for (i, param_name) in action.param_names.iter().enumerate() {
        if let Some(name) = param_name
            && let Some(val) = arg_vals.get(i)
        {
            env.define(name, *val);
        }
    }

    // Salvar arena original e trocar para fiber_arena (já é fiber_arena,
    // mas call_action_body cria sua própria arena — precisamos evitar isso).
    // Em vez de chamar call_action_body (que cria nova arena), executamos
    // o body diretamente com a fiber_arena.
    let mut result = 0i64;
    for stmt in &action.body {
        match eval(&mut ctx, stmt, &mut env) {
            Ok(v) => result = v,
            Err(InterpError::Return(v)) => {
                result = v;
                break;
            }
            Err(e) => {
                eprintln!(
                    "interp_trampoline: erro na action '{}': {e}",
                    entry.action_name
                );
                env.pop_scope();
                return 0;
            }
        }
    }

    env.pop_scope();
    result
}

/// Avalia `fork!(action, args)` — spawn de fiber.
///
/// Registra a action na tabela global, empacota `(action_id, args_ptr, n_args)`
/// na arena, e chama `kata_rt_spawn(rt, interp_trampoline, caller_arena, packed_ptr)`.
///
/// Retorna Unit — fork é fire-and-forget (structured concurrency garante
/// que o parent espera os filhos).
pub(crate) fn eval_fork(
    ctx: &mut InterpCtx,
    action_name: &str,
    args: &kata_ast::Spanned<kata_inference::TypedExpr>,
    env: &mut Env,
) -> Result<i64, InterpError> {
    // 1. Avaliar args (tupla) → args_ptr.
    let args_ptr = eval(ctx, args, env)?;

    // 2. Contar n_args e coletar arg_tys para despacho de overloads.
    let (n_args, arg_tys) = match &args.node.kind {
        TypedExprKind::Tuple { elements } => (
            elements.len() as i64,
            elements.iter().map(|e| e.node.ty.clone()).collect(),
        ),
        TypedExprKind::Unit => (0, Vec::new()),
        _ => (1, vec![args.node.ty.clone()]),
    };

    // 3. Registrar action na tabela global → action_id.
    let action_id = register_action(action_name, &ctx.module, arg_tys, ctx.enum_registry.clone());

    // 4. Empacotar (action_id, args_ptr, n_args) na arena.
    let packed_ptr = kata_rt::kata_rt_arena_alloc(ctx.rt_ptr, ctx.arena, 24);
    unsafe {
        std::ptr::write(packed_ptr as *mut i64, action_id);
        std::ptr::write((packed_ptr as *mut i64).add(1), args_ptr);
        std::ptr::write((packed_ptr as *mut i64).add(2), n_args);
    }

    // 5. kata_rt_spawn(rt, fn_ptr, caller_arena, packed_ptr).
    let fn_ptr = interp_trampoline as *const () as i64;
    let caller_arena = ctx.arena;
    let _fiber_id = kata_rt::kata_rt_spawn(ctx.rt_ptr, fn_ptr, caller_arena, packed_ptr);

    Ok(0) // Unit
}

/// Avalia `spawn!(action, args)` — spawn de processo OS via fork.
///
/// Similar ao fork! mas chama `kata_rt_spawn_process(rt, fn_ptr, args_ptr, arena)`.
pub(crate) fn eval_spawn(
    ctx: &mut InterpCtx,
    action_name: &str,
    args: &kata_ast::Spanned<kata_inference::TypedExpr>,
    env: &mut Env,
) -> Result<i64, InterpError> {
    // 1. Avaliar args (tupla) → args_ptr.
    let args_ptr = eval(ctx, args, env)?;

    // 2. Contar n_args e coletar arg_tys.
    let (n_args, arg_tys) = match &args.node.kind {
        TypedExprKind::Tuple { elements } => (
            elements.len() as i64,
            elements.iter().map(|e| e.node.ty.clone()).collect(),
        ),
        TypedExprKind::Unit => (0, Vec::new()),
        _ => (1, vec![args.node.ty.clone()]),
    };

    // 3. Registrar action na tabela global → action_id.
    let action_id = register_action(action_name, &ctx.module, arg_tys, ctx.enum_registry.clone());

    // 4. Empacotar (action_id, args_ptr, n_args) na arena.
    let packed_ptr = kata_rt::kata_rt_arena_alloc(ctx.rt_ptr, ctx.arena, 24);
    unsafe {
        std::ptr::write(packed_ptr as *mut i64, action_id);
        std::ptr::write((packed_ptr as *mut i64).add(1), args_ptr);
        std::ptr::write((packed_ptr as *mut i64).add(2), n_args);
    }

    // 5. kata_rt_spawn_process(rt, fn_ptr, args_ptr, arena).
    let fn_ptr = interp_trampoline as *const () as i64;
    let arena = ctx.arena;
    let _ = kata_rt::kata_rt_spawn_process(ctx.rt_ptr, fn_ptr, packed_ptr, arena);

    Ok(0) // Unit
}
