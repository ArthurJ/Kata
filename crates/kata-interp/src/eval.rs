//! Eval — dispatch central sobre `TypedExprKind`.
//!
//! Cada variante de `TypedExprKind` é avaliada chamando a função de
//! runtime apropriada diretamente. O interpretador não re-parseia,
//! não re-infere, não re-resolve. A TAST já tem tipos resolvidos,
//! dispatch decidido, escape analysis marcado, TRMA aplicado.

use std::ffi::CString;
use std::sync::Arc;

use kata_ast::Spanned;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::{
    CacheStrategy, ChannelKind, TypedExpr, TypedExprKind, TypedLambdaClause, TypedModule,
    TypedPattern, TypedSelectArm,
};
use kata_rt as rt;

use crate::env::Env;
use crate::ffi_dispatch::ffi_dispatch;
use crate::value::{Value, decode_smi, encode_smi, f64_to_value, fits_smi};

/// Erro de interpretação — control flow + erros runtime.
#[derive(Debug)]
pub enum InterpError {
    /// Erro genuíno de execução.
    Runtime(String),
    /// `return expr` — propagado para o nível da action.
    Return(Value),
    /// `break` — propagado para o nível do loop.
    Break,
    /// `continue` — propagado para o nível do loop.
    Continue,
}

impl std::fmt::Display for InterpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InterpError::Runtime(msg) => write!(f, "{msg}"),
            InterpError::Return(_) => write!(f, "return não capturado"),
            InterpError::Break => write!(f, "break não capturado"),
            InterpError::Continue => write!(f, "continue não capturado"),
        }
    }
}

impl std::error::Error for InterpError {}

/// Contexto de execução do interpretador.
pub struct InterpCtx {
    /// Runtime pointer (Box<Runtime> como i64).
    pub(crate) rt_ptr: i64,
    /// Handle da fiber arena atual.
    pub(crate) arena: i64,
    /// TypedModule (Arc'd) — sobrevive durante toda a execução.
    pub(crate) module: Arc<TypedModule>,
    /// Enum registry — para mapear tag → nome de variante em show_sum.
    pub(crate) enum_registry: Arc<kata_core::EnumRegistry>,
}

impl InterpCtx {
    /// Cria o contexto de execução a partir de um `TypedModule` com enum registry.
    ///
    /// Este é o construtor raiz — todos os outros delegam para este.
    pub fn new_with_registry(
        module: TypedModule,
        rt_ptr: i64,
        enum_registry: Arc<kata_core::EnumRegistry>,
    ) -> Self {
        // Registrar rt_ptr em TLS — FFIs de coleção (list_cons, array_alloc,
        // dict_insert, etc.) lêem o Runtime via `rt_ptr()` (thread_local),
        // não via parâmetro. Sem isto, essas FFIs veem 0 → null deref.
        rt::set_rt_ptr(rt_ptr);

        let arena = rt::kata_rt_arena_create(rt_ptr);

        InterpCtx {
            rt_ptr,
            arena,
            module: Arc::new(module),
            enum_registry,
        }
    }

    /// Cria o contexto de execução com enum registry default (vazio).
    ///
    /// Mantido para compatibilidade — `interpret` e `interpret_with_env`
    /// usam este quando não têm enum_registry disponível.
    pub fn new(module: TypedModule, rt_ptr: i64) -> Self {
        Self::new_with_registry(module, rt_ptr, Arc::new(kata_core::EnumRegistry::new()))
    }

    /// Cria o contexto reusando uma arena existente (para fibers do scheduler).
    ///
    /// O `arena` é o handle da arena criada pelo scheduler para o fiber.
    /// O `module` é compartilhado via Arc (clonado da tabela global).
    pub fn new_with_arena_registry(
        module: Arc<TypedModule>,
        rt_ptr: i64,
        arena: i64,
        enum_registry: Arc<kata_core::EnumRegistry>,
    ) -> Self {
        rt::set_rt_ptr(rt_ptr);
        InterpCtx {
            rt_ptr,
            arena,
            module,
            enum_registry,
        }
    }

    /// Cria o contexto reusando uma arena existente com enum registry default.
    ///
    /// Compatibilidade — delega para `new_with_arena_registry` com registry vazio.
    pub fn new_with_arena(module: Arc<TypedModule>, rt_ptr: i64, arena: i64) -> Self {
        Self::new_with_arena_registry(
            module,
            rt_ptr,
            arena,
            Arc::new(kata_core::EnumRegistry::new()),
        )
    }

    /// Avalia o entry point do módulo criando um Env novo.
    pub fn eval_entry(&mut self) -> Result<Value, InterpError> {
        let mut env = Env::new();
        self.eval_entry_with_env(&mut env)
    }

    /// Avalia o entry point reusando um `Env` persistente (para REPL).
    ///
    /// O Env deve conter bindings `let` acumulados de linhas anteriores.
    /// Constants e pre_entry são avaliados no env fornecido.
    pub fn eval_entry_with_env(&mut self, env: &mut Env) -> Result<Value, InterpError> {
        // Clonar Arc<TypedModule> para evitar borrow conflict: iteramos
        // sobre &module.* (imutável) enquanto chamamos eval(self, ...) (mutável).
        let module = self.module.clone();

        // Bindings stdio: __stdin__/__stdout__/__stderr__ são handles File.
        // O resolution os define no type_env; o interpretador precisa
        // definir no env de runtime com os handles reais.
        // Usar define_if_undefined para não sobrescrever bindings persistentes.
        if env.lookup("__stdin__").is_none() {
            env.define("__stdin__", rt::kata_rt_stdin());
        }
        if env.lookup("__stdout__").is_none() {
            env.define("__stdout__", rt::kata_rt_stdout());
        }
        if env.lookup("__stderr__").is_none() {
            env.define("__stderr__", rt::kata_rt_stderr());
        }

        // Avaliar constants (ConstantBinding) no prólogo.
        for c in &module.constants {
            if let TypedExprKind::ConstantBinding { name, value } = &c.node.kind {
                let v = eval(self, value, env)?;
                env.define(name, v);
            }
        }

        // Avaliar pre_entry (let bindings top-level antes do entry).
        for stmt in &module.pre_entry {
            eval(self, stmt, env)?;
        }

        // Avaliar entry point.
        //
        // Scheduler mode (espelha o JIT): se o entry point é uma ActionCall
        // definida pelo usuário (sem ffi_symbol), faz spawn + run em vez de
        // call_action direto. O fiber raiz executa a action dentro do
        // scheduler de fibers, permitindo que fork! dentro da action crie
        // fibers filhas que são drenadas pelo run.
        //
        // FFIs (echo!, sleep!, etc.) continuam sendo call direto.
        if let TypedExprKind::ActionCall {
            callee,
            args,
            ffi_symbol: None,
            ..
        } = &module.entry.node.kind
        {
            return eval_entry_scheduler_mode(self, callee, args, env);
        }

        eval(self, &module.entry, env)
    }
}

/// Avalia o entry point em scheduler mode (spawn + run).
///
/// Espelha o comportamento do codegen em `scheduler_mode = true`:
/// 1. `kata_rt_scheduler_init(rt)` → root_arena (no-op se já inicializado)
/// 2. Registrar a action na tabela global → action_id
/// 3. Avaliar args (tupla) → args_ptr na arena
/// 4. Empacotar (action_id, args_ptr, n_args) na arena
/// 5. `kata_rt_spawn(rt, interp_trampoline, caller_arena, packed_ptr)`
/// 6. `kata_rt_run(rt)` → resultado do fiber raiz
fn eval_entry_scheduler_mode(
    ctx: &mut InterpCtx,
    callee: &str,
    args: &kata_ast::Spanned<kata_inference::TypedExpr>,
    env: &mut Env,
) -> Result<Value, InterpError> {
    // 1. Inicializar scheduler (compatibilidade — na prática é no-op,
    //    mas seta rt_ptr em TLS).
    let root_arena = rt::kata_rt_scheduler_init(ctx.rt_ptr);

    // 2. Avaliar args (tupla) → args_ptr na root_arena.
    //    O entry point usa root_arena como caller_arena.
    let saved_arena = ctx.arena;
    ctx.arena = root_arena;
    let args_ptr = eval(ctx, args, env)?;
    let (n_args, arg_tys) = match &args.node.kind {
        TypedExprKind::Tuple { elements } => (
            elements.len() as i64,
            elements.iter().map(|e| e.node.ty.clone()).collect(),
        ),
        TypedExprKind::Unit => (0, Vec::new()),
        _ => (1, vec![args.node.ty.clone()]),
    };

    // 3. Registrar action na tabela global → action_id.
    let action_id =
        crate::csp::register_action(callee, &ctx.module, arg_tys, ctx.enum_registry.clone());

    // 4. Empacotar (action_id, args_ptr, n_args) na arena.
    let packed_ptr = rt::kata_rt_arena_alloc(ctx.rt_ptr, ctx.arena, 24);
    unsafe {
        std::ptr::write(packed_ptr as *mut i64, action_id);
        std::ptr::write((packed_ptr as *mut i64).add(1), args_ptr);
        std::ptr::write((packed_ptr as *mut i64).add(2), n_args);
    }

    // 5. kata_rt_spawn(rt, fn_ptr, caller_arena, packed_ptr).
    let fn_ptr = crate::csp::interp_trampoline as *const () as i64;
    let caller_arena = ctx.arena;
    let _fiber_id = rt::kata_rt_spawn(ctx.rt_ptr, fn_ptr, caller_arena, packed_ptr);

    // 6. kata_rt_run(rt) → resultado do fiber raiz.
    let result = rt::kata_rt_run(ctx.rt_ptr);

    // Restaurar arena original.
    ctx.arena = saved_arena;

    // Verificar sentinelas.
    if result == rt::DEADLOCK_SENTINEL {
        return Err(InterpError::Runtime(
            "deadlock detectado pelo scheduler".to_string(),
        ));
    }
    if result == rt::TIMEOUT_SENTINEL {
        return Err(InterpError::Runtime("timeout do scheduler".to_string()));
    }

    Ok(result)
}

/// Avalia uma expressão tipada.
pub fn eval(
    ctx: &mut InterpCtx,
    expr: &Spanned<TypedExpr>,
    env: &mut Env,
) -> Result<Value, InterpError> {
    match &expr.node.kind {
        // ── Literais ─────────────────────────────────────────
        TypedExprKind::IntLit { text } => {
            // Parsear texto para i64. Se cabe em SMI, inline.
            // Se não, usar kata_rt_tag_int (aloca BigInt).
            let cleaned = text.replace('_', "");
            let (sign, digits) = if let Some(rest) = cleaned.strip_prefix('-') {
                (-1i64, rest)
            } else if let Some(rest) = cleaned.strip_prefix('+') {
                (1i64, rest)
            } else {
                (1i64, cleaned.as_str())
            };

            let n = if let Some(hex) = digits
                .strip_prefix("0x")
                .or_else(|| digits.strip_prefix("0X"))
            {
                i64::from_str_radix(hex, 16).ok()
            } else if let Some(oct) = digits
                .strip_prefix("0o")
                .or_else(|| digits.strip_prefix("0O"))
            {
                i64::from_str_radix(oct, 8).ok()
            } else if let Some(bin) = digits
                .strip_prefix("0b")
                .or_else(|| digits.strip_prefix("0B"))
            {
                i64::from_str_radix(bin, 2).ok()
            } else {
                digits.parse::<i64>().ok()
            };

            match n {
                Some(val) if fits_smi(val * sign) => Ok(encode_smi(val * sign)),
                _ => {
                    // BigInt — usar tag_int_from_str do runtime
                    let cstr = CString::new(text.as_str()).unwrap();
                    let val = rt::kata_rt_tag_int_from_str(cstr.as_ptr(), text.len() as i64);
                    Ok(val)
                }
            }
        }
        TypedExprKind::FloatLit { text } => {
            let f: f64 = text.parse().unwrap_or(0.0);
            Ok(f64_to_value(f))
        }
        TypedExprKind::TextLit { text } => {
            // Alocar C string na heap ( CString::into_raw )
            let cstr = CString::new(text.as_str()).unwrap_or_else(|_| CString::new("").unwrap());
            Ok(cstr.into_raw() as i64)
        }
        TypedExprKind::BytesLit { bytes } => {
            // Alocar Bytes no runtime
            let ptr = unsafe {
                rt::kata_rt_bytes_from_ptr(bytes.as_ptr() as i64, bytes.len() as i64, ctx.arena)
            };
            Ok(ptr)
        }
        TypedExprKind::Unit => Ok(0),

        // ── Identificador ────────────────────────────────────
        TypedExprKind::Ident { name } => env
            .lookup(name)
            .ok_or_else(|| InterpError::Runtime(format!("variável não definida: {name}"))),

        // ── Let / Var / Reassign ─────────────────────────────
        TypedExprKind::Let { name, value } => {
            let v = eval(ctx, value, env)?;
            env.define(name, v);
            Ok(0)
        }
        TypedExprKind::LetDestruct {
            temp_name,
            value,
            bindings,
        } => {
            let v = eval(ctx, value, env)?;
            env.define(temp_name, v);
            for (name, field_expr) in bindings {
                let fv = eval(ctx, field_expr, env)?;
                env.define(name, fv);
            }
            Ok(0)
        }
        TypedExprKind::Var { name, value } => {
            let v = eval(ctx, value, env)?;
            env.define(name, v);
            Ok(0)
        }
        TypedExprKind::Reassign { name, value } => {
            let v = eval(ctx, value, env)?;
            env.reassign(name, v).map_err(InterpError::Runtime)?;
            Ok(0)
        }

        // ── Grouping ─────────────────────────────────────────
        TypedExprKind::Grouping { inner } => eval(ctx, inner, env),

        // ── TypeAscription ───────────────────────────────────
        // O typeck já validou a ascription. Em runtime, precisamos
        // converter o valor quando o tipo alvo é diferente do tipo
        // interno (ex: Int → Rational precisa chamar FFI).
        TypedExprKind::TypeAscription {
            expr: inner,
            target_ty,
            ..
        } => {
            let inner_ty = &inner.node.ty;
            // Mesmo tipo — no-op.
            if inner_ty == target_ty {
                return eval(ctx, inner, env);
            }
            // IntLit → Float: reinterpretar como f64.
            if let TypedExprKind::IntLit { ref text } = inner.node.kind {
                if matches!(target_ty, Ty::Prim(PrimTy::Float)) {
                    let val: f64 = text.parse().unwrap_or(f64::NAN);
                    return Ok(f64_to_value(val));
                }
                // IntLit → Rational: chamar kata_rt_rat_literal.
                if matches!(target_ty, Ty::Prim(PrimTy::Rational)) {
                    let cstr =
                        CString::new(text.as_str()).unwrap_or_else(|_| CString::new("0").unwrap());
                    let ptr = cstr.as_ptr();
                    let len = text.len() as i64;
                    let result = unsafe { rt::kata_rt_rat_literal(ptr, len) };
                    std::mem::forget(cstr); // não drop — ponteiro bruto
                    return Ok(result as i64);
                }
            }
            // FloatLit → Rational: chamar kata_rt_rat_literal.
            if let TypedExprKind::FloatLit { ref text } = inner.node.kind
                && matches!(target_ty, Ty::Prim(PrimTy::Rational))
            {
                let cstr =
                    CString::new(text.as_str()).unwrap_or_else(|_| CString::new("0").unwrap());
                let ptr = cstr.as_ptr();
                let len = text.len() as i64;
                let result = unsafe { rt::kata_rt_rat_literal(ptr, len) };
                std::mem::forget(cstr);
                return Ok(result as i64);
            }
            // Refined/alias ascription (Int → PositiveInt, etc.) — no-op.
            // O typeck já validou os predicados. O valor em runtime é o mesmo.
            if matches!(target_ty, Ty::Struct(_)) {
                return eval(ctx, inner, env);
            }
            // Downcast refined → base (PositiveInt → Int, Altura → Float) —
            // mesmos bits, no-op.
            if matches!(inner_ty, Ty::Struct(_)) {
                return eval(ctx, inner, env);
            }
            // Fallback: avaliar inner.
            eval(ctx, inner, env)
        }

        // ── Tuple / Struct ───────────────────────────────────
        TypedExprKind::Tuple { elements } => {
            // Aloca N*8 na arena, store cada elemento
            let n = elements.len() as i64;
            let ptr = rt::kata_rt_arena_alloc(ctx.rt_ptr, ctx.arena, n * 8);
            for (i, elem) in elements.iter().enumerate() {
                let v = eval(ctx, elem, env)?;
                unsafe {
                    std::ptr::write((ptr as *mut Value).add(i), v);
                }
            }
            Ok(ptr)
        }
        TypedExprKind::StructConstruct { values, .. } => {
            let n = values.len() as i64;
            let ptr = rt::kata_rt_arena_alloc(ctx.rt_ptr, ctx.arena, n * 8);
            for (i, elem) in values.iter().enumerate() {
                let v = eval(ctx, elem, env)?;
                unsafe {
                    std::ptr::write((ptr as *mut Value).add(i), v);
                }
            }
            Ok(ptr)
        }
        TypedExprKind::FieldAccess {
            expr: inner,
            field_index,
            ..
        } => {
            let ptr = eval(ctx, inner, env)?;
            Ok(unsafe { std::ptr::read((ptr as *const Value).add(*field_index as usize)) })
        }
        TypedExprKind::IndexAccess {
            expr: inner,
            element_index,
            ..
        } => {
            let ptr = eval(ctx, inner, env)?;
            Ok(unsafe { std::ptr::read((ptr as *const Value).add(*element_index as usize)) })
        }

        // ── Closure (chamada de função) ──────────────────────
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            let arg_vals: Vec<Value> = args
                .iter()
                .map(|a| eval(ctx, a, env))
                .collect::<Result<_, _>>()?;

            if let Some(sym) = ffi_symbol {
                // show sintetizado: interceptar antes do ffi_dispatch.
                // O ffi_dispatch não tem acesso ao Ty do valor, mas aqui
                // temos o TypedExpr do argumento com seu tipo resolvido.
                if sym.starts_with("__kata_show__") {
                    // show recebe 1 arg; pegar seu tipo do TypedExpr
                    let arg_ty = &args[0].node.ty;
                    Ok(crate::show::show_value(arg_vals[0], arg_ty, ctx))
                } else {
                    // Dispatch FFI direto
                    ffi_dispatch(sym, &arg_vals, ctx.rt_ptr, ctx.arena)
                        .map_err(InterpError::Runtime)
                }
            } else {
                // Chamada de função Kata pura
                let call_arg_tys: Vec<Ty> = args.iter().map(|a| a.node.ty.clone()).collect();
                call_named_function(ctx, callee, &arg_vals, &call_arg_tys, env)
            }
        }

        // ── Lambda (construção de closure) ───────────────────
        TypedExprKind::Lambda {
            clauses, captures, ..
        } => {
            // Construir closure value na arena:
            // offset 0: tag CLOSURE_TAG
            // offset 1: ptr para cláusulas (Vec<TypedLambdaClause> na heap Rust)
            // offset 2: ptr para captures (Vec<(String, Value)> na heap Rust)
            let size = 24; // 3 words
            let ptr = rt::kata_rt_arena_alloc(ctx.rt_ptr, ctx.arena, size);

            // Armazenar cláusulas num Box na heap Rust
            let clauses_box = Box::new(clauses.clone());
            let clauses_ptr = Box::into_raw(clauses_box) as i64;

            // Armazenar captures (nome + valor) num Box na heap Rust
            let mut cap_pairs: Vec<(String, Value)> = Vec::new();
            for cap in captures.iter() {
                let cap_val = env.lookup(&cap.name).ok_or_else(|| {
                    InterpError::Runtime(format!("capture não encontrada: {}", cap.name))
                })?;
                cap_pairs.push((cap.name.clone(), cap_val));
            }
            let captures_box = Box::new(cap_pairs);
            let captures_ptr = Box::into_raw(captures_box) as i64;

            unsafe {
                std::ptr::write(ptr as *mut Value, CLOSURE_TAG);
                std::ptr::write((ptr as *mut Value).add(1), clauses_ptr);
                std::ptr::write((ptr as *mut Value).add(2), captures_ptr);
            }
            Ok(ptr)
        }

        // ── Match ────────────────────────────────────────────
        TypedExprKind::Match { scrutinee, arms } => {
            let scrut_val = eval(ctx, scrutinee, env)?;
            for arm in arms {
                // ── Escopo único da action (Impl D): braço NÃO abre
                // escopo filho. Snapshot das chaves ANTES do pattern;
                // bindings frescos do braço evaporam no fim (reuso
                // de `var` externo persiste com o novo valor).
                let keys = env.scope_keys();
                let matched = if let Some(ref pat) = arm.pattern {
                    match_pattern(pat, scrut_val, env)
                } else {
                    true // otherwise
                };
                if matched {
                    // Avaliar guard se existir
                    if let Some(ref guard) = arm.guard {
                        let guard_val = eval(ctx, guard, env)?;
                        if guard_val == 0 {
                            env.evaporate(keys);
                            continue;
                        }
                    }
                    let result = eval(ctx, &arm.body, env);
                    env.evaporate(keys);
                    return result;
                }
                env.evaporate(keys);
            }
            Err(InterpError::Runtime("match não-exaustivo".to_string()))
        }

        // ── Block ────────────────────────────────────────────
        // Escopo único da action: Block não abre escopo. Statements
        // definem no escopo atual (typeck garante evaporação de
        // bindings de braço via undefine; Block puro não tem
        // bindings próprios além dos statements).
        TypedExprKind::Block { stmts } => {
            let mut result = 0i64;
            for stmt in stmts {
                result = eval(ctx, stmt, env)?;
            }
            Ok(result)
        }

        // ── ActionCall ───────────────────────────────────────
        TypedExprKind::ActionCall {
            callee,
            args,
            ffi_symbol,
            ..
        } => {
            // Extrair argumentos diretamente da TAST em vez de avaliar
            // como Tuple e depois desempacotar. O `args` é sempre uma
            // Tuple na TAST (ou Unit para sem args).
            // Guardamos também o Ty de cada arg para conversão via show.
            let mut arg_vals: Vec<Value> = Vec::new();
            let mut arg_tys: Vec<Ty> = Vec::new();
            match &args.node.kind {
                TypedExprKind::Tuple { elements } => {
                    for elem in elements {
                        arg_vals.push(eval(ctx, elem, env)?);
                        arg_tys.push(elem.node.ty.clone());
                    }
                }
                TypedExprKind::Unit => {}
                // Fallback: avaliar como valor único
                _ => {
                    arg_vals.push(eval(ctx, args, env)?);
                    arg_tys.push(args.node.ty.clone());
                }
            }

            // echo!/println! recebem Text (C string ptr). O typeck insere
            // `show` para tipos não-Text antes de chamar a FFI. O interpretador
            // precisa fazer o mesmo: converter cada arg via show_value.
            if ffi_symbol.as_deref() == Some("kata_rt_print")
                || ffi_symbol.as_deref() == Some("kata_rt_println")
            {
                let mut converted = Vec::with_capacity(arg_vals.len());
                for (val, ty) in arg_vals.iter().zip(arg_tys.iter()) {
                    if matches!(ty, Ty::Prim(PrimTy::Text)) {
                        converted.push(*val);
                    } else {
                        converted.push(crate::show::show_value(*val, ty, ctx));
                    }
                }
                return ffi_dispatch(
                    ffi_symbol.as_ref().unwrap(),
                    &converted,
                    ctx.rt_ptr,
                    ctx.arena,
                )
                .map_err(InterpError::Runtime);
            }

            if let Some(sym) = ffi_symbol {
                ffi_dispatch(sym, &arg_vals, ctx.rt_ptr, ctx.arena).map_err(InterpError::Runtime)
            } else {
                // Action definida pelo usuário
                call_action(ctx, callee, &arg_vals, &arg_tys, env)
            }
        }

        // ── TypeOf ───────────────────────────────────────────
        TypedExprKind::TypeOf { expr: inner } => {
            let ty_str = format!("{}", inner.node.ty);
            let cstr = CString::new(ty_str).unwrap_or_else(|_| CString::new("?").unwrap());
            Ok(cstr.into_raw() as i64)
        }

        // ── Controle de fluxo ────────────────────────────────
        TypedExprKind::Return(expr) => {
            let v = eval(ctx, expr, env)?;
            Err(InterpError::Return(v))
        }
        TypedExprKind::Break => Err(InterpError::Break),
        TypedExprKind::Continue => Err(InterpError::Continue),

        TypedExprKind::Loop { body } => loop {
            // ── Escopo único da action (Impl D): iteração NÃO abre
            // escopo. Snapshot ANTES do corpo; bindings frescos da
            // iteração evaporam no fim dela (antes de processar
            // Break/Continue — não vazam para a próxima iteração).
            let keys = env.scope_keys();
            let mut result = 0i64;
            let mut early_exit: Option<Result<Value, InterpError>> = None;
            for stmt in body {
                match eval(ctx, stmt, env) {
                    Ok(v) => result = v,
                    Err(InterpError::Break) => {
                        early_exit = Some(Err(InterpError::Break));
                        break;
                    }
                    Err(InterpError::Continue) => {
                        early_exit = Some(Err(InterpError::Continue));
                        break;
                    }
                    Err(e) => {
                        early_exit = Some(Err(e));
                        break;
                    }
                }
            }
            env.evaporate(keys);
            match early_exit {
                Some(Err(InterpError::Break)) => return Ok(result),
                Some(Err(InterpError::Continue)) => continue,
                Some(Err(e)) => return Err(e),
                Some(Ok(_)) => return Ok(result),
                None => {}
            }
        },

        // ── Variants ─────────────────────────────────────────
        TypedExprKind::VariantQual { enum_name, tag, .. } => {
            // Boolean é i64 cru (1=True, 0=False), não Sum box.
            if enum_name == "Boolean" {
                return Ok(1 - *tag as i64); // True(0)→1, False(1)→0
            }
            Ok(rt::kata_rt_store_sum_result(*tag as i64, 0, ctx.arena))
        }
        TypedExprKind::VariantConstruct { tag, payload, .. } => {
            let payload_val = eval(ctx, payload, env)?;
            Ok(rt::kata_rt_store_sum_result(
                *tag as i64,
                payload_val,
                ctx.arena,
            ))
        }

        // ── List literal ─────────────────────────────────────
        TypedExprKind::ListLit { elements } => {
            let mut list = rt::kata_rt_list_nil();
            for elem in elements.iter().rev() {
                let v = eval(ctx, elem, env)?;
                list = rt::kata_rt_list_cons(v, list, ctx.arena);
            }
            Ok(list)
        }

        // ── Array literal ────────────────────────────────────
        TypedExprKind::ArrayLit { elements } => {
            let n = elements.len() as i64;
            let ptr = rt::kata_rt_array_alloc(n, ctx.arena);
            for (i, elem) in elements.iter().enumerate() {
                let v = eval(ctx, elem, env)?;
                rt::kata_rt_array_set(ptr, i as i64, v);
            }
            Ok(ptr)
        }

        // ── In (membership) ──────────────────────────────────
        TypedExprKind::In { item, collection } => {
            let item_val = eval(ctx, item, env)?;
            let coll_val = eval(ctx, collection, env)?;
            // Heurística: se coll_val parece ser uma List (cons cell),
            // usar list_contains; se Array, array_contains.
            // Por enquanto, tentar list primeiro.
            Ok(rt::kata_rt_list_contains(coll_val, item_val))
        }

        // ── ConstantBinding ──────────────────────────────────
        TypedExprKind::ConstantBinding { name, value } => {
            let v = eval(ctx, value, env)?;
            env.define(name, v);
            Ok(0)
        }

        // ── ForIn ────────────────────────────────────────────
        TypedExprKind::ForIn {
            var_name,
            iterable,
            body,
            ..
        } => {
            let coll_val = eval(ctx, iterable, env)?;
            // Iterar lista (Cons cells)
            // ── Escopo único da action (Impl D): iteração NÃO abre
            // escopo. O loop-var define no escopo da action — reuso
            // (var externo prévio) persiste com o último elemento;
            // fresco evapora no fim de CADA iteração.
            let mut current = coll_val;
            while current != 0 {
                let head = rt::kata_rt_list_head(current);
                let tail = rt::kata_rt_list_tail(current);
                let keys = env.scope_keys();
                env.define(var_name, head);
                for stmt in body {
                    eval(ctx, stmt, env)?;
                }
                env.evaporate(keys);
                current = tail;
            }
            Ok(0)
        }

        // ── HOFs: Map, Filter, Fold, FusedStream ─────────────
        TypedExprKind::Map {
            callback,
            collection,
            ..
        } => {
            let coll_val = eval(ctx, collection, env)?;
            let mut result = rt::kata_rt_list_nil();
            // Coletar resultados e construir lista reversa
            let mut items = Vec::new();
            let mut current = coll_val;
            while current != 0 {
                let head = rt::kata_rt_list_head(current);
                let tail = rt::kata_rt_list_tail(current);
                let mapped = call_closure(ctx, callback, &[head], env)?;
                items.push(mapped);
                current = tail;
            }
            // Construir lista na ordem correta
            for v in items.into_iter().rev() {
                result = rt::kata_rt_list_cons(v, result, ctx.arena);
            }
            Ok(result)
        }
        TypedExprKind::Filter {
            callback,
            collection,
            ..
        } => {
            let coll_val = eval(ctx, collection, env)?;
            let mut items = Vec::new();
            let mut current = coll_val;
            while current != 0 {
                let head = rt::kata_rt_list_head(current);
                let tail = rt::kata_rt_list_tail(current);
                let keep = call_closure(ctx, callback, &[head], env)?;
                if keep != 0 {
                    items.push(head);
                }
                current = tail;
            }
            let mut result = rt::kata_rt_list_nil();
            for v in items.into_iter().rev() {
                result = rt::kata_rt_list_cons(v, result, ctx.arena);
            }
            Ok(result)
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            let coll_val = eval(ctx, collection, env)?;
            let mut acc = eval(ctx, initial, env)?;
            let mut current = coll_val;
            while current != 0 {
                let head = rt::kata_rt_list_head(current);
                let tail = rt::kata_rt_list_tail(current);
                acc = call_closure(ctx, callback, &[acc, head], env)?;
                current = tail;
            }
            Ok(acc)
        }
        TypedExprKind::FusedStream { stages, source, .. } => {
            let coll_val = eval(ctx, source, env)?;
            let mut items = Vec::new();
            let mut current = coll_val;
            while current != 0 {
                let head = rt::kata_rt_list_head(current);
                let tail = rt::kata_rt_list_tail(current);
                let mut val = head;
                let mut keep = true;
                for stage in stages {
                    match stage {
                        kata_inference::FusedStage::Filter { callback, .. } => {
                            let pass = call_closure(ctx, callback, &[val], env)?;
                            if pass == 0 {
                                keep = false;
                                break;
                            }
                        }
                        kata_inference::FusedStage::Map { callback, .. } => {
                            val = call_closure(ctx, callback, &[val], env)?;
                        }
                    }
                }
                if keep {
                    items.push(val);
                }
                current = tail;
            }
            let mut result = rt::kata_rt_list_nil();
            for v in items.into_iter().rev() {
                result = rt::kata_rt_list_cons(v, result, ctx.arena);
            }
            Ok(result)
        }

        // ── RangeLit ─────────────────────────────────────────
        TypedExprKind::RangeLit {
            start,
            step,
            end,
            inclusive,
            ..
        } => {
            // Construir lista materializada do range
            // (interpretador não tem range lazy — materializa)
            let start_val = decode_smi(eval(ctx, start, env)?);
            let step_val = decode_smi(eval(ctx, step, env)?);
            let end_val = decode_smi(eval(ctx, end, env)?);

            let mut items = Vec::new();
            if step_val > 0 {
                let limit = if *inclusive { end_val + 1 } else { end_val };
                let mut i = start_val;
                while i < limit {
                    items.push(encode_smi(i));
                    i += step_val;
                }
            } else if step_val < 0 {
                let limit = if *inclusive { end_val - 1 } else { end_val };
                let mut i = start_val;
                while i > limit {
                    items.push(encode_smi(i));
                    i += step_val;
                }
            }
            let mut result = rt::kata_rt_list_nil();
            for v in items.into_iter().rev() {
                result = rt::kata_rt_list_cons(v, result, ctx.arena);
            }
            Ok(result)
        }

        // ── HeapSnapshot ─────────────────────────────────────
        TypedExprKind::HeapSnapshot { snapshot_id, .. } => {
            Ok(rt::kata_rt_get_snapshot(*snapshot_id as i64))
        }

        // ── CSP (Fase 5) ─────────────────────────────────────
        TypedExprKind::ChannelCreate {
            kind,
            elem_ty: _,
            cross_process: false,
        } => {
            // Criar canal conforme o kind. Alocar tupla (handle, handle) na arena.
            let handle = match kind {
                ChannelKind::Rendezvous => rt::kata_rt_channel_create(ctx.arena),
                ChannelKind::Buffered(cap) => {
                    rt::kata_rt_queue_create(ctx.arena, *cap, 0) // policy=Block
                }
                ChannelKind::Broadcast => rt::kata_rt_broadcast_create(ctx.arena),
            };
            if handle == 0 {
                return Err(InterpError::Runtime("falha ao criar canal".to_string()));
            }
            // Alocar tupla (handle, handle) — 16 bytes.
            let ptr = rt::kata_rt_arena_alloc(ctx.rt_ptr, ctx.arena, 16);
            unsafe {
                std::ptr::write(ptr as *mut Value, handle);
                std::ptr::write((ptr as *mut Value).add(1), handle);
            }
            Ok(ptr)
        }
        TypedExprKind::ChannelCreate {
            cross_process: true,
            ..
        } => Err(InterpError::Runtime(
            "canais cross-process não suportados no interpretador".to_string(),
        )),
        TypedExprKind::ChannelSend { channel, value } => {
            let handle = eval(ctx, channel, env)?;
            let val = eval(ctx, value, env)?;
            let result = rt::kata_rt_channel_send(handle, val);
            if result < 0 {
                // WOULD_BLOCK — sem fiber, não pode bloquear.
                return Err(InterpError::Runtime(
                    "channel_send bloqueado (sem fiber disponível)".to_string(),
                ));
            }
            Ok(0) // Unit
        }
        TypedExprKind::ChannelRecv {
            channel,
            recv_ty: _,
            bind_name,
        } => {
            let handle = eval(ctx, channel, env)?;
            let val = rt::kata_rt_channel_recv(handle);
            if val < 0 && val != 0 {
                // WOULD_BLOCK ou erro — sem fiber, não pode bloquear.
                // Mas val=0 é um valor válido (Unit, False, etc).
                // Verificar se é WOULD_BLOCK especificamente.
                // kata_rt_channel_recv retorna WOULD_BLOCK (-1) se não há dado.
                return Err(InterpError::Runtime(
                    "channel_recv sem dado disponível (sem fiber)".to_string(),
                ));
            }
            env.define(bind_name, val);
            Ok(val)
        }
        TypedExprKind::ReceiverFactoryCall {
            factory,
            elem_ty: _,
        } => {
            let factory_handle = eval(ctx, factory, env)?;
            let rx_handle = rt::kata_rt_broadcast_receiver_create(ctx.arena, factory_handle);
            if rx_handle == 0 {
                return Err(InterpError::Runtime(
                    "falha ao criar receiver do broadcast".to_string(),
                ));
            }
            Ok(rx_handle)
        }
        TypedExprKind::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => eval_select(
            ctx,
            arms,
            timeout_ms.as_deref(),
            timeout_body.as_deref(),
            env,
        ),
        TypedExprKind::Fork {
            action_name,
            action_expr: _,
            args,
        } => crate::csp::eval_fork(ctx, action_name, args, env),
        TypedExprKind::Spawn {
            action_name,
            action_expr: _,
            args,
        } => crate::csp::eval_spawn(ctx, action_name, args, env),

        // ── Dict / Set literais ─────────────────────────────
        TypedExprKind::DictLit {
            entries,
            key_ty,
            value_ty: _,
        } => {
            let (hash_fn, eq_fn) = resolve_hash_eq(key_ty)?;
            let mut dict = rt::kata_rt_dict_empty(ctx.arena);
            for (key_expr, val_expr) in entries {
                let key_val = eval(ctx, key_expr, env)?;
                let val_val = eval(ctx, val_expr, env)?;
                let hash = hash_fn(key_val);
                dict = rt::kata_rt_dict_insert(dict, key_val, val_val, hash, eq_fn, ctx.arena);
            }
            Ok(dict)
        }
        TypedExprKind::SetLit { elements, elem_ty } => {
            let (hash_fn, eq_fn) = resolve_hash_eq(elem_ty)?;
            let mut set = rt::kata_rt_set_empty(ctx.arena);
            for elem in elements {
                let v = eval(ctx, elem, env)?;
                let hash = hash_fn(v);
                set = rt::kata_rt_set_insert(set, v, hash, eq_fn, ctx.arena);
            }
            Ok(set)
        }
    }
}

/// Sentinelas de canal (espelham channel/select.rs).
const WOULD_BLOCK: i64 = -1;
const SELECT_TIMEOUT: i64 = -2;

/// Avalia `select` com braços de canal e timeout.
///
/// Para o interpretador (sem scheduler de fibers), o select é síncrono:
/// chama `kata_rt_select` que tenta todos os canais sem bloquear. Se
/// nenhum tem dado e há timeout, espera via `std::thread::sleep`. Se
/// nenhum tem dado e não há timeout, retorna erro.
fn eval_select(
    ctx: &mut InterpCtx,
    arms: &[TypedSelectArm],
    timeout_ms: Option<&Spanned<TypedExpr>>,
    timeout_body: Option<&Spanned<TypedExpr>>,
    env: &mut Env,
) -> Result<Value, InterpError> {
    // Coletar braços de canal (ignorar IoRead por enquanto).
    let channel_arms: Vec<&TypedSelectArm> = arms
        .iter()
        .filter(|a| matches!(a, TypedSelectArm::Channel { .. }))
        .collect();

    if channel_arms.is_empty() {
        // Sem canais — apenas timeout.
        if let Some(body) = timeout_body {
            return eval(ctx, body, env);
        }
        return Err(InterpError::Runtime(
            "select sem braços de canal".to_string(),
        ));
    }

    // Avaliar handles dos canais.
    let mut handles: Vec<i64> = Vec::with_capacity(channel_arms.len());
    for arm in &channel_arms {
        if let TypedSelectArm::Channel { channel, .. } = arm {
            handles.push(eval(ctx, channel, env)?);
        }
    }

    // Avaliar timeout_ms (se houver).
    let timeout_val = if let Some(tm_expr) = timeout_ms {
        Some(eval(ctx, tm_expr, env)?)
    } else {
        None
    };

    // Alocar array de handles na arena.
    let n = handles.len() as i64;
    let handles_ptr = rt::kata_rt_arena_alloc(ctx.rt_ptr, ctx.arena, n * 8);
    for (i, &h) in handles.iter().enumerate() {
        unsafe {
            std::ptr::write((handles_ptr as *mut i64).add(i), h);
        }
    }

    // Chamar kata_rt_select.
    // O timeout_ms na TAST é um Int SMI-tagged. A FFI espera o valor cru
    // em ms — decodificar SMI (val >> 1) como o codegen faz (ushr_imm 1).
    let timeout_ms_val = match timeout_val {
        Some(v) => decode_smi(v),
        None => 0,
    };
    let result = rt::kata_rt_select(handles_ptr as *const i64, n, timeout_ms_val);

    if result >= 0 {
        // Canal pronto — result é o índice.
        let idx = result as usize;
        if let Some(TypedSelectArm::Channel {
            bind_name, body, ..
        }) = channel_arms.get(idx)
        {
            // Fazer o recv.
            let handle = handles[idx];
            let val = rt::kata_rt_channel_recv(handle);
            if val == WOULD_BLOCK {
                return Err(InterpError::Runtime(
                    "select: canal pronto mas recv falhou".to_string(),
                ));
            }
            // ── Escopo único da action (Impl D): braço de select define
            // no escopo da action; binding fresco evapora no fim do
            // braço (typeck rejeita leitura pós-select).
            let keys = env.scope_keys();
            env.define(bind_name, val);
            let result = eval(ctx, body, env);
            env.evaporate(keys);
            return result;
        }
        return Err(InterpError::Runtime(
            "select: índice de braço inválido".to_string(),
        ));
    }

    if result == SELECT_TIMEOUT {
        if let Some(body) = timeout_body {
            return eval(ctx, body, env);
        }
        return Ok(0); // Unit se não há timeout_body
    }

    // WOULD_BLOCK — sem fiber, não pode bloquear.
    Err(InterpError::Runtime(
        "select: nenhum canal pronto (sem fiber/scheduler)".to_string(),
    ))
}

/// Tag para closures na arena (magic number para identificar struct de closure).
const CLOSURE_TAG: i64 = 0x0C10_5C10;

/// Chama uma função nomeada (declarada com `::` e `lambda`).
fn call_named_function(
    ctx: &mut InterpCtx,
    callee: &Spanned<TypedExpr>,
    args: &[Value],
    arg_tys: &[Ty],
    env: &mut Env,
) -> Result<Value, InterpError> {
    // Se o callee é um Ident, procurar na tabela de funções
    if let TypedExprKind::Ident { name } = &callee.node.kind {
        // Clonar o nome para evitar borrow de callee enquanto emprestamos ctx
        let name = name.clone();
        // Procurar nas funções nomeadas do módulo.
        // Clonar Arc<TypedModule> para evitar borrow conflict.
        let module = ctx.module.clone();
        // Coletar todos os overloads com o nome e aridade correspondentes.
        let n_args = args.len();
        let func_candidates: Vec<_> = module
            .functions
            .iter()
            .filter(|f| f.name == name && f.param_types.len() == n_args)
            .collect();
        if !func_candidates.is_empty() {
            let best = if func_candidates.len() == 1 {
                func_candidates[0]
            } else {
                // Despachar por tipo (overloads): melhor ty_compat_score.
                func_candidates
                    .iter()
                    .max_by_key(|f| ty_compat_score(arg_tys, &f.param_types))
                    .copied()
                    .expect("func_candidates é não-vazio")
            };
            let clauses = best.clauses.clone();
            // ── @cache (deferred do escopo-plano): memoização no interp.
            // Espelha o JIT — mesma API TLS do runtime. Hit retorna sem
            // reexecutar o body; miss executa e insere.
            if let Some(spec) = best.cache_spec.as_ref() {
                // fn_id estável dentro do run: índice em module.functions
                // (caches TLS são por-processo — não cruzam com o JIT).
                let fn_id = module
                    .functions
                    .iter()
                    .position(|f| f.name == best.name && f.param_types == best.param_types)
                    .map(|i| i as i64)
                    .unwrap_or_else(|| fnv1a(best.name.as_bytes()));
                let handle = rt::kata_rt_cache_get_or_create(
                    ctx.arena,
                    fn_id,
                    spec.capacity,
                    match spec.strategy {
                        CacheStrategy::LRU => 0,
                        CacheStrategy::FIFO => 1,
                        CacheStrategy::MRU => 2,
                        CacheStrategy::LFU => 3,
                    },
                );
                // Serializar args por conteúdo: Int → bits LE; Float → bits do
                // f64; Text → bytes do C-string. Tipos compostos por hora não
                // são cacheados (miss → executa, sem insert — conservador).
                let mut key: Vec<u8> = Vec::with_capacity(64);
                let mut cacheable = true;
                for (ty, &val) in best.param_types.iter().zip(args.iter()) {
                    serialize_key_part(ty, val, &mut key, &mut cacheable);
                }
                if cacheable && !key.is_empty() {
                    let hit =
                        rt::kata_rt_cache_lookup(handle, key.as_ptr() as i64, key.len() as i64);
                    if hit != 0 {
                        // Hit: synthetic_pre (diretivas Enter — @log{enter} etc.)
                        // dispara MESMO em hit, espelhando o wrapper do JIT
                        // (log roda antes do cache lookup). Body não roda.
                        if !clauses.is_empty() && !clauses[0].synthetic_pre.is_empty() {
                            env.push_scope();
                            for (i, val) in args.iter().enumerate() {
                                env.define(&format!("__param_{i}"), *val);
                            }
                            for pre_expr in &clauses[0].synthetic_pre {
                                eval(ctx, pre_expr, env)?;
                            }
                            env.pop_scope();
                        }
                        return Ok(hit);
                    }
                    let result = call_typed_clauses(ctx, &clauses, args.to_vec(), env)?;
                    rt::kata_rt_cache_insert(handle, key.as_ptr() as i64, key.len() as i64, result);
                    return Ok(result);
                }
            }
            return call_typed_clauses(ctx, &clauses, args.to_vec(), env);
        }
        // Procurar nas actions do módulo (despachar por aridade + tipo)
        let n_args = args.len();
        let candidates: Vec<&kata_inference::TypedAction> = module
            .actions
            .iter()
            .filter(|a| a.name == name && a.param_types.len() == n_args)
            .collect();
        if !candidates.is_empty() {
            // Coletar arg_tys do callee (TypedExpr de cada arg)
            let arg_tys: Vec<Ty> = if let TypedExprKind::Closure { args, .. } = &callee.node.kind {
                args.iter().map(|a| a.node.ty.clone()).collect()
            } else {
                vec![]
            };
            let best = if candidates.len() == 1 {
                candidates[0]
            } else {
                candidates
                    .iter()
                    .max_by_key(|a| ty_compat_score(&arg_tys, &a.param_types))
                    .copied()
                    .expect("candidates é não-vazio")
            };
            let action_clone = best.clone();
            return call_action_body(ctx, &action_clone, args, env);
        }
        // Procurar no env (lambda como valor)
        if let Some(val) = env.lookup(&name) {
            return call_closure_value(ctx, val, args, env);
        }
        return Err(InterpError::Runtime(format!(
            "função/action não encontrada: {name}"
        )));
    }
    // Se é uma lambda anônima (Lambda), avaliar para obter a closure value
    let closure_val = eval(ctx, callee, env)?;
    call_closure_value(ctx, closure_val, args, env)
}

/// Chama uma closure armazenada na arena (struct com tag CLOSURE_TAG).
fn call_closure_value(
    ctx: &mut InterpCtx,
    closure_val: Value,
    args: &[Value],
    env: &mut Env,
) -> Result<Value, InterpError> {
    let ptr = closure_val as *const Value;
    let tag = unsafe { std::ptr::read(ptr) };
    if tag != CLOSURE_TAG {
        return Err(InterpError::Runtime(format!(
            "valor não é uma closure (tag={:#x})",
            tag
        )));
    }
    let clauses_ptr = unsafe { std::ptr::read(ptr.add(1)) } as *mut Vec<TypedLambdaClause>;
    let captures_ptr = unsafe { std::ptr::read(ptr.add(2)) } as *mut Vec<(String, Value)>;

    // Reconstruir cláusulas e captures.
    let clauses = unsafe { &*clauses_ptr };
    let captures = unsafe { &*captures_ptr };

    // Empilhar escopo e definir captures com seus nomes.
    env.push_scope();
    for (name, val) in captures.iter() {
        env.define(name, *val);
    }

    let result = call_typed_clauses(ctx, clauses, args.to_vec(), env);
    env.pop_scope();
    result
}

/// Resultado da avaliação em posição de cauda.
///
/// `Done(v)` — expressão produziu valor final.
/// `TailCall(name, args)` — expressão é uma chamada de cauda direta para
/// função Kata pura nomeada. O trampoline em `call_typed_clauses` faz
/// loop com os novos argumentos em vez de recursar.
enum TailResult {
    Done(Value),
    TailCall { name: String, args: Vec<Value> },
}

/// Avalia uma expressão em **posição de cauda**.
///
/// Idêntica a `eval` para todos os nós, exceto:
/// - `Closure { callee: Ident, ffi_symbol: None }` → retorna `TailCall`
///   em vez de chamar `call_named_function` recursivamente.
/// - `Match` → chama `eval_tail` no body do arm matching (não `eval`).
/// - `Block` → chama `eval_tail` na última expressão (não `eval`).
///
/// Isso permite que o trampoline em `call_typed_clauses` faça TCO:
/// em vez de empilhar frames Rust para cada chamada de cauda, faz loop.
fn eval_tail(
    ctx: &mut InterpCtx,
    expr: &Spanned<TypedExpr>,
    env: &mut Env,
) -> Result<TailResult, InterpError> {
    match &expr.node.kind {
        // ── Closure (chamada de função) em posição de cauda ──
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            if ffi_symbol.is_none() {
                // Chamada de função Kata pura — retornar TailCall.
                if let TypedExprKind::Ident { name } = &callee.node.kind {
                    let arg_vals: Vec<Value> = args
                        .iter()
                        .map(|a| eval(ctx, a, env))
                        .collect::<Result<_, _>>()?;
                    return Ok(TailResult::TailCall {
                        name: name.clone(),
                        args: arg_vals,
                    });
                }
            }
            // Fallback: não é chamada de cauda otimizável — avaliar normalmente.
            let v = eval(ctx, expr, env)?;
            Ok(TailResult::Done(v))
        }

        // ── Match em posição de cauda — propagar eval_tail para o arm ──
        TypedExprKind::Match { scrutinee, arms } => {
            let scrut_val = eval(ctx, scrutinee, env)?;
            for arm in arms {
                // ── Escopo único da action (Impl D): braço NÃO abre
                // escopo filho. Snapshot das chaves ANTES do pattern;
                // bindings frescos do braço evaporam no fim (reuso
                // de `var` externo persiste com o novo valor).
                let keys = env.scope_keys();
                let matched = if let Some(ref pat) = arm.pattern {
                    match_pattern(pat, scrut_val, env)
                } else {
                    true // otherwise
                };
                if matched {
                    if let Some(ref guard) = arm.guard {
                        let guard_val = eval(ctx, guard, env)?;
                        if guard_val == 0 {
                            env.evaporate(keys);
                            continue;
                        }
                    }
                    let result = eval_tail(ctx, &arm.body, env);
                    env.evaporate(keys);
                    return result;
                }
                env.evaporate(keys);
            }
            Err(InterpError::Runtime("match não-exaustivo".to_string()))
        }

        // ── Block em posição de cauda — propagar eval_tail para a última expr ──
        // Escopo único da action: sem push/pop.
        TypedExprKind::Block { stmts } => {
            if stmts.is_empty() {
                return Ok(TailResult::Done(0));
            }
            let last = stmts.len() - 1;
            for (i, stmt) in stmts.iter().enumerate() {
                if i == last {
                    let result = eval_tail(ctx, stmt, env);
                    return result;
                }
                eval(ctx, stmt, env)?;
            }
            unreachable!("loop sempre retorna no último stmt")
        }

        // ── Demais nós — delegar para eval ──
        _ => {
            let v = eval(ctx, expr, env)?;
            Ok(TailResult::Done(v))
        }
    }
}

// ── @cache: serialização de key por conteúdo ──────────────────────

/// FNV-1a — mesmo hash do `canonical_fn_id` do codegen.
fn fnv1a(bytes: &[u8]) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}

/// Serializa um argumento da cache key por CONTEÚDO (não por ponteiro).
///
/// - Int: 8 bytes LE dos bits do valor
/// - Float: 8 bytes dos bits do f64
/// - Text: len (4 bytes LE) + bytes do C-string (terminador excluído)
///
/// Tipos compostos (List/Struct/Tuple/Sum) marcam `cacheable = false` —
/// o interp executa sem insert (miss permanente, conservador). O JIT
/// cobre via type descriptor; paridade futura.
fn serialize_key_part(ty: &Ty, val: Value, key: &mut Vec<u8>, cacheable: &mut bool) {
    match ty {
        Ty::Prim(PrimTy::Int) | Ty::Prim(PrimTy::Rational) => {
            key.extend_from_slice(&val.to_le_bytes());
        }
        Ty::Prim(PrimTy::Float) => {
            key.extend_from_slice(&val.to_le_bytes());
        }
        Ty::Prim(PrimTy::Text) => {
            if val == 0 {
                key.extend_from_slice(&0u32.to_le_bytes());
                return;
            }
            let cstr = unsafe { std::ffi::CStr::from_ptr(val as *const _) };
            let bytes = cstr.to_bytes();
            key.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            key.extend_from_slice(bytes);
        }
        _ => *cacheable = false,
    }
}

/// Chama um conjunto de cláusulas tipadas com argumentos.
///
/// Implementa TCO via trampoline: quando o body da cláusula (ou guard body)
/// é uma chamada de cauda direta (`eval_tail` retorna `TailCall`), faz loop
/// com os novos argumentos em vez de recursar. Isso evita stack overflow
/// em recursão de cauda (incluindo TRMA rewrite).
fn call_typed_clauses(
    ctx: &mut InterpCtx,
    clauses: &[TypedLambdaClause],
    mut args: Vec<Value>,
    env: &mut Env,
) -> Result<Value, InterpError> {
    // Trampoline: loop até produzir um valor final.
    // Em cada iteração, avalia o body da cláusula matching com `eval_tail`.
    // Se `eval_tail` retorna `TailCall`, resolve a função e faz loop
    // com as novas cláusulas e argumentos — sem empilhar frames Rust.
    let mut current_clauses: Vec<TypedLambdaClause> = clauses.to_vec();

    // synthetic_pre (diretivas Enter): avalia uma vez antes do trampoline.
    // synthetic_post (diretivas Exit): avalia uma vez após o trampoline.
    // Ambos precisam de __param_{i} no escopo para _args.
    let has_synthetic = !clauses.is_empty()
        && (!clauses[0].synthetic_pre.is_empty() || !clauses[0].synthetic_post.is_empty());

    if has_synthetic {
        env.push_scope();
        for (i, val) in args.iter().enumerate() {
            env.define(&format!("__param_{i}"), *val);
        }
        for pre_expr in &clauses[0].synthetic_pre {
            eval(ctx, pre_expr, env)?;
        }
    }

    let result = 'trampoline: loop {
        // Avaliar cláusulas atuais. Se o body é uma chamada de cauda,
        // `tail_call` recebe o nome e args para o próximo loop.
        let mut tail_call: Option<(String, Vec<Value>)> = None;

        'clause_loop: for clause in &current_clauses {
            env.push_scope();

            for (i, val) in args.iter().enumerate() {
                env.define(&format!("__param_{i}"), *val);
            }

            let mut all_match = true;
            for (i, pat) in clause.patterns.iter().enumerate() {
                if !match_pattern(pat, args[i], env) {
                    all_match = false;
                    break;
                }
            }

            if all_match {
                for wb in &clause.with_bindings {
                    let v = eval(ctx, &wb.value, env)?;
                    env.define(&wb.name, v);
                }

                if !clause.guards.is_empty() {
                    for guard in &clause.guards {
                        let matched = if let Some(ref cond) = guard.condition {
                            eval(ctx, cond, env)? != 0
                        } else {
                            true // otherwise
                        };
                        if matched {
                            let result = eval_tail(ctx, &guard.body, env);
                            env.pop_scope();
                            match result? {
                                TailResult::Done(v) => break 'trampoline Ok(v),
                                TailResult::TailCall {
                                    name,
                                    args: new_args,
                                } => {
                                    tail_call = Some((name, new_args));
                                    break 'clause_loop;
                                }
                            }
                        }
                    }
                    // Nenhum guard passou — tentar próxima cláusula
                    env.pop_scope();
                    continue;
                }

                // Sem guards — avaliar body com eval_tail
                let result = eval_tail(ctx, &clause.body, env);
                env.pop_scope();
                match result? {
                    TailResult::Done(v) => break 'trampoline Ok(v),
                    TailResult::TailCall {
                        name,
                        args: new_args,
                    } => {
                        tail_call = Some((name, new_args));
                        break 'clause_loop;
                    }
                }
            }
            env.pop_scope();
        }

        match tail_call {
            Some((name, new_args)) => {
                let (new_clauses, resolved_args) = resolve_tail_call(ctx, &name, new_args, env)?;
                current_clauses = new_clauses;
                args = resolved_args;
            }
            None => {
                break 'trampoline Err(InterpError::Runtime(format!(
                    "nenhuma cláusula匹配: args={args:?}"
                )));
            }
        }
    };

    // synthetic_post (diretivas Exit): avalia uma vez após o trampoline
    // produzir o valor final, com _return bindado ao resultado.
    if has_synthetic {
        if !clauses[0].synthetic_post.is_empty() {
            let result_val = result?;
            env.define("_return", result_val);
            for post_expr in &clauses[0].synthetic_post {
                eval(ctx, post_expr, env)?;
            }
            env.pop_scope();
            Ok(result_val)
        } else {
            // Só synthetic_pre — pop_scope e retornar.
            env.pop_scope();
            result
        }
    } else {
        result
    }
}

/// Resolve um TailCall: encontra as cláusulas da função nomeada.
///
/// Retorna `(clauses, args)` para o trampoline continuar o loop.
/// `eval_tail` só produz `TailCall` para `Closure { callee: Ident,
/// ffi_symbol: None }` — ou seja, chamadas de função Kata pura nomeada.
/// Actions (ActionCall) e closures anônimas (callee = Lambda) não
/// produzem `TailCall` e são avaliadas recursivamente por `eval`.
fn resolve_tail_call(
    ctx: &mut InterpCtx,
    name: &str,
    args: Vec<Value>,
    _env: &mut Env,
) -> Result<(Vec<TypedLambdaClause>, Vec<Value>), InterpError> {
    let module = ctx.module.clone();
    let n_args = args.len();

    let func_candidates: Vec<_> = module
        .functions
        .iter()
        .filter(|f| f.name == name && f.param_types.len() == n_args)
        .collect();
    if func_candidates.is_empty() {
        return Err(InterpError::Runtime(format!(
            "resolve_tail_call: função não encontrada: {name}"
        )));
    }
    let best = if func_candidates.len() == 1 {
        func_candidates[0]
    } else {
        // Despachar por tipo (overloads): melhor ty_compat_score.
        let arg_tys = func_candidates[0].param_types.clone();
        func_candidates
            .iter()
            .max_by_key(|f| ty_compat_score(&arg_tys, &f.param_types))
            .copied()
            .expect("func_candidates é não-vazio")
    };
    Ok((best.clauses.clone(), args))
}

/// Chama um callback (closure lambda) com argumentos.
fn call_closure(
    ctx: &mut InterpCtx,
    callback: &Spanned<TypedExpr>,
    args: &[Value],
    env: &mut Env,
) -> Result<Value, InterpError> {
    // O callback é uma Lambda ou Ident
    let closure_val = eval(ctx, callback, env)?;
    call_closure_value(ctx, closure_val, args, env)
}

/// Chama uma action definida pelo usuário.
/// Chama uma action definida pelo usuário.
///
/// Despacha por nome E compatibilidade de tipos (aridade + Ty dos params).
/// Múltiplos overloads (ex: log, _log_publish, write) têm o mesmo nome mas
/// assinaturas diferentes. O codegen usa DispatchTable com chave
/// `(name, params, ret)`; o interp usa comparação de Ty por compatibilidade.
fn call_action(
    ctx: &mut InterpCtx,
    name: &str,
    args: &[Value],
    arg_tys: &[Ty],
    env: &mut Env,
) -> Result<Value, InterpError> {
    let module = ctx.module.clone();
    let n_args = args.len();

    // Coletar todos os overloads com mesma aridade.
    let candidates: Vec<&kata_inference::TypedAction> = module
        .actions
        .iter()
        .filter(|a| a.name == name && a.param_types.len() == n_args)
        .collect();

    if candidates.is_empty() {
        return Err(InterpError::Runtime(format!(
            "action não encontrada: {name} (aridade {n_args})"
        )));
    }

    // Se há apenas um candidato, despachar diretamente.
    if candidates.len() == 1 {
        let action_clone = candidates[0].clone();
        return call_action_body(ctx, &action_clone, args, env);
    }

    // Múltiplos overloads com mesma aridade — despachar por tipo.
    // Comparar arg_tys vs param_types com Ty::ty_compat (compatibilidade
    // estrutural, não igualdade estrita — permite Generic com type args
    // e Sum com variantes).
    let best = candidates
        .iter()
        .max_by_key(|a| ty_compat_score(arg_tys, &a.param_types))
        .copied()
        .expect("candidates é não-vazio");

    let action_clone = best.clone();
    call_action_body(ctx, &action_clone, args, env)
}

/// Pontua compatibilidade de tipos entre args e params.
/// Retorna número de matches exatos + parciais. Maior = melhor.
pub(crate) fn ty_compat_score(arg_tys: &[Ty], param_types: &[Ty]) -> usize {
    arg_tys
        .iter()
        .zip(param_types.iter())
        .map(|(arg, param)| {
            if arg == param {
                3 // match exato
            } else if ty_compatible(arg, param) {
                2 // compatível (Generic com type args, etc.)
            } else {
                0 // incompatível
            }
        })
        .sum()
}

/// Verifica compatibilidade flexível entre dois Ty.
/// Permite Generic com type args diferentes (ex: Result(File, Text) vs Result(Socket, Text))
/// e Prim vs Prim iguais.
fn ty_compatible(a: &Ty, b: &Ty) -> bool {
    // Prim iguais
    if a == b {
        return true;
    }
    // Generic com mesmo nome mas type args diferentes — contar como compatível
    if let (Ty::Generic(name_a, args_a), Ty::Generic(name_b, args_b)) = (a, b) {
        return name_a == name_b && args_a.len() == args_b.len();
    }
    // Sum com mesmo nome — compatível (variantes diferentes do mesmo enum)
    if let (Ty::Sum(name_a), Ty::Sum(name_b)) = (a, b) {
        return name_a == name_b;
    }
    // File vs Text — incompatível (caso comum: log 3-arg)
    // Nada mais a fazer sem InterfaceRegistry
    false
}

/// Executa o corpo de uma action.
fn call_action_body(
    ctx: &mut InterpCtx,
    action: &kata_inference::TypedAction,
    args: &[Value],
    env: &mut Env,
) -> Result<Value, InterpError> {
    // Criar fiber arena para a action
    let action_arena = rt::kata_rt_arena_create(ctx.rt_ptr);

    env.push_scope();

    // Bindar argumentos aos parâmetros
    for (i, param_name) in action.param_names.iter().enumerate() {
        if let Some(name) = param_name {
            env.define(name, args[i]);
        }
    }
    // Se params são posicionais (None), bindar por posição com nomes genéricos
    // Mas na verdade, actions posicionais não têm nomes de params no env.
    // O body da action usa os nomes dos params. Se param_names é None,
    // precisamos de outra forma. Mas para Fase 1, as actions canônicas
    // (main, echo, etc.) têm params nomeados.

    // Salvar arena original e trocar para action_arena
    let saved_arena = ctx.arena;
    ctx.arena = action_arena;

    let mut result = 0i64;
    let mut early_return = None;
    for stmt in &action.body {
        match eval(ctx, stmt, env) {
            Ok(v) => result = v,
            Err(InterpError::Return(v)) => {
                result = v;
                early_return = Some(());
                break;
            }
            Err(e) => {
                ctx.arena = saved_arena;
                env.pop_scope();
                return Err(e);
            }
        }
    }

    ctx.arena = saved_arena;
    env.pop_scope();

    // Destruir a action arena
    rt::kata_rt_arena_destroy(ctx.rt_ptr, action_arena);

    let _ = early_return;
    Ok(result)
}

/// Faz pattern matching de um valor contra um pattern tipado.
fn match_pattern(pat: &Spanned<TypedPattern>, value: Value, env: &mut Env) -> bool {
    match &pat.node {
        TypedPattern::Ident { name, .. } => {
            env.define(name, value);
            true
        }
        TypedPattern::Wildcard => true,
        TypedPattern::Literal { value: lit_expr } => {
            // Avaliar o literal e comparar
            // O literal é um TypedExpr — podemos comparar o valor bruto
            // Para Int: comparar SMI. Para Float: comparar bits.
            // Como o literal já é tipado, avaliamos e comparamos.
            // Mas não temos ctx aqui — precisamos de uma forma de
            // comparar sem avaliar. Para literais simples (Int, Float,
            // Text), o valor é direto.
            //
            // Hack: para Fase 1, comparar SMI diretamente se o ty é Int.
            // Para Float, comparar bits. Para Text, comparar ponteiros
            // ( CString — comparar conteúdo).
            //
            // Melhor: avaliar o literal_expr (que é sempre constante)
            // e comparar o valor. Mas match_pattern não tem ctx.
            //
            // Solução: passar ctx para match_pattern. Vamos refatorar.
            //
            // Por enquanto, comparação bruta:
            match &lit_expr.node.kind {
                TypedExprKind::IntLit { text } => {
                    let cleaned = text.replace('_', "");
                    let n = cleaned.parse::<i64>().unwrap_or(0);
                    value == encode_smi(n)
                }
                TypedExprKind::FloatLit { text } => {
                    let f: f64 = text.parse().unwrap_or(0.0);
                    value == f64_to_value(f)
                }
                TypedExprKind::TextLit { text } => {
                    // Comparar C strings
                    let val_cstr =
                        unsafe { std::ffi::CStr::from_ptr(value as *const std::os::raw::c_char) };
                    val_cstr.to_string_lossy() == *text
                }
                TypedExprKind::Unit => value == 0,
                // F5.5: `rational N` em pattern — Closure com ffi_symbol
                // "kata_rt_int_to_rational". Constrói o valor Rational e
                // compara com kata_rt_rat_eq (comparação estrutural, não bitwise).
                TypedExprKind::Closure {
                    ffi_symbol, args, ..
                } if ffi_symbol.as_deref() == Some("kata_rt_int_to_rational") => {
                    // Extrai o Int do argumento (IntLit).
                    let n = if let Some(TypedExprKind::IntLit { text }) =
                        args.first().map(|a| &a.node.kind)
                    {
                        let cleaned = text.replace('_', "");
                        cleaned.parse::<i64>().unwrap_or(0)
                    } else {
                        return false;
                    };
                    // kata_rt_int_to_rational espera SMI-tagged (faz is_smi
                    // internamente).
                    let lit_rat = unsafe { rt::kata_rt_int_to_rational(encode_smi(n)) }
                        as *const num_rational::BigRational;
                    let val_rat = value as *const num_rational::BigRational;
                    unsafe { rt::kata_rt_rat_eq(val_rat, lit_rat) == 1 }
                }
                _ => false,
            }
        }
        TypedPattern::Variant {
            enum_name,
            tag,
            sub_patterns,
            ..
        } => {
            // Boolean é representado como i64 cru (1=True, 0=False),
            // não como Sum box (ponteiro para arena). Caso especial.
            if enum_name == "Boolean" {
                // True (tag=0) → value==1, False (tag=1) → value==0
                if value != (1 - *tag as i64) {
                    return false;
                }
                // Boolean não tem payload
                return true;
            }

            let actual_tag = rt::kata_rt_sum_tag_int(value);
            if actual_tag != *tag as i64 {
                return false;
            }
            if let Some(subs) = sub_patterns {
                let payload = unsafe { std::ptr::read((value as *const Value).add(1)) };
                for sub in subs {
                    if !match_pattern(sub, payload, env) {
                        return false;
                    }
                }
            }
            true
        }
        TypedPattern::Cons { head, tail } => {
            if value == 0 {
                return false; // Nil
            }
            let head_val = rt::kata_rt_list_head(value);
            let tail_val = rt::kata_rt_list_tail(value);
            match_pattern(head, head_val, env) && match_pattern(tail, tail_val, env)
        }
        TypedPattern::Nil => value == 0,
        TypedPattern::Tuple { elements } => {
            for (i, ep) in elements.iter().enumerate() {
                let elem = unsafe { std::ptr::read((value as *const Value).add(i)) };
                if !match_pattern(ep, elem, env) {
                    return false;
                }
            }
            true
        }
    }
}

/// Resolve hash_fn e eq_fn para um tipo de chave/elemento de Dict/Set.
///
/// Retorna (hash_fn, eq_fn_ptr) onde hash_fn é uma closure que chama a
/// função FFI de hashing apropriada, e eq_fn_ptr é o ponteiro bruto
/// como i64 (para passar para kata_rt_dict_insert / kata_rt_set_insert).
///
/// Espelha `dict_set_lit::hash_fn_name` / `eq_fn_name` do codegen.
#[allow(clippy::type_complexity)]
fn resolve_hash_eq(ty: &Ty) -> Result<(Box<dyn Fn(i64) -> i64>, i64), InterpError> {
    match ty {
        Ty::Prim(PrimTy::Int) => Ok((
            Box::new(|v| rt::kata_rt_hash_int(v)),
            rt::kata_rt_bi_eq as *const () as i64,
        )),
        Ty::Prim(PrimTy::Text) => Ok((
            Box::new(|v| rt::kata_rt_hash_text(v)),
            rt::kata_rt_string_eq as *const () as i64,
        )),
        Ty::Prim(PrimTy::Rational) => Ok((
            Box::new(|v| rt::kata_rt_hash_rational(v)),
            rt::kata_rt_bi_eq as *const () as i64,
        )),
        _ => Err(InterpError::Runtime(format!(
            "Dict/Set literal: tipo não-hashable: {ty}"
        ))),
    }
}
