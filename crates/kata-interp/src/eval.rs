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
    ChannelKind, TypedExpr, TypedExprKind, TypedLambdaClause, TypedModule, TypedPattern,
    TypedSelectArm,
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
    rt_ptr: i64,
    /// Handle da fiber arena atual.
    arena: i64,
    /// TypedModule (Arc'd) — sobrevive durante toda a execução.
    module: Arc<TypedModule>,
}

impl InterpCtx {
    /// Cria o contexto de execução a partir de um `TypedModule`.
    pub fn new(module: TypedModule, rt_ptr: i64) -> Self {
        // Registrar rt_ptr em TLS — FFIs de coleção (list_cons, array_alloc,
        // dict_insert, etc.) lêem o Runtime via `rt_ptr()` (thread_local),
        // não via parâmetro. Sem isto, essas FFIs veem 0 → null deref.
        rt::set_rt_ptr(rt_ptr);

        let arena = rt::kata_rt_arena_create(rt_ptr);

        InterpCtx {
            rt_ptr,
            arena,
            module: Arc::new(module),
        }
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
        eval(self, &module.entry, env)
    }
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
        TypedExprKind::TypeAscription { expr: inner, .. } => eval(ctx, inner, env),

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
                call_named_function(ctx, callee, &arg_vals, env)
            }
        }

        // ── Lambda (construção de closure) ───────────────────
        TypedExprKind::Lambda {
            clauses, captures, ..
        } => {
            // Construir closure value na arena:
            // offset 0: tag CLOSURE_TAG
            // offset 1: ptr para cláusulas (vec na heap Rust)
            // offset 2: n_captures
            // offset 3..: captures[]
            let n_caps = captures.len() as i64;
            let size = 24 + n_caps * 8; // 3 words + captures
            let ptr = rt::kata_rt_arena_alloc(ctx.rt_ptr, ctx.arena, size);

            // Armazenar cláusulas num Arc na heap Rust
            let clauses_box = Box::new(clauses.clone());
            let clauses_ptr = Box::into_raw(clauses_box) as i64;

            unsafe {
                std::ptr::write(ptr as *mut Value, CLOSURE_TAG);
                std::ptr::write((ptr as *mut Value).add(1), clauses_ptr);
                std::ptr::write((ptr as *mut Value).add(2), n_caps);
                for (i, cap) in captures.iter().enumerate() {
                    let cap_val = env.lookup(&cap.name).ok_or_else(|| {
                        InterpError::Runtime(format!("capture não encontrada: {}", cap.name))
                    })?;
                    std::ptr::write((ptr as *mut Value).add(3 + i), cap_val);
                }
            }
            Ok(ptr)
        }

        // ── Match ────────────────────────────────────────────
        TypedExprKind::Match { scrutinee, arms } => {
            let scrut_val = eval(ctx, scrutinee, env)?;
            for arm in arms {
                env.push_scope();
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
                            env.pop_scope();
                            continue;
                        }
                    }
                    let result = eval(ctx, &arm.body, env);
                    env.pop_scope();
                    return result;
                }
                env.pop_scope();
            }
            Err(InterpError::Runtime("match não-exaustivo".to_string()))
        }

        // ── Block ────────────────────────────────────────────
        TypedExprKind::Block { stmts } => {
            env.push_scope();
            let mut result = 0i64;
            for stmt in stmts {
                result = eval(ctx, stmt, env)?;
            }
            env.pop_scope();
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
                call_action(ctx, callee, &arg_vals, env)
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
            env.push_scope();
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
            env.pop_scope();
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
            let mut current = coll_val;
            while current != 0 {
                let head = rt::kata_rt_list_head(current);
                let tail = rt::kata_rt_list_tail(current);
                env.push_scope();
                env.define(var_name, head);
                for stmt in body {
                    eval(ctx, stmt, env)?;
                }
                env.pop_scope();
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
                return Err(InterpError::Runtime(format!(
                    "channel_send bloqueado (sem fiber disponível)"
                )));
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
        TypedExprKind::Fork { .. } => Err(InterpError::Runtime(
            "fork! não implementado no interpretador (requer scheduler de fibers)".to_string(),
        )),
        TypedExprKind::Spawn { .. } => Err(InterpError::Runtime(
            "spawn! não implementado no interpretador (requer fork OS)".to_string(),
        )),

        // ── Dict / Set (Fase 2 — stubs por enquanto) ─────────
        TypedExprKind::DictLit { entries, .. } => {
            // Fase 2: implementar via kata_rt_dict_empty + insert
            let dict = rt::kata_rt_dict_empty(ctx.arena);
            for (key_expr, val_expr) in entries {
                let key_val = eval(ctx, key_expr, env)?;
                let val_val = eval(ctx, val_expr, env)?;
                // TODO: hash e eq_fn precisam do tipo
                // Por enquanto, stub
                let _ = (key_val, val_val);
            }
            Ok(dict)
        }
        TypedExprKind::SetLit { elements, .. } => {
            let set = rt::kata_rt_set_empty(ctx.arena);
            for elem in elements {
                let v = eval(ctx, elem, env)?;
                // TODO: hash e eq_fn precisam do tipo
                let _ = v;
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
    let timeout_ms_val = timeout_val.unwrap_or(0);
    let result = rt::kata_rt_select(handles_ptr as *const i64, n, timeout_ms_val);

    if result >= 0 {
        // Canal pronto — result é o índice.
        let idx = result as usize;
        if let Some(TypedSelectArm::Channel {
            channel,
            bind_name,
            body,
            ..
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
            env.define(bind_name, val);
            return eval(ctx, body, env);
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
const CLOSURE_TAG: i64 = 0xC105_C10;

/// Chama uma função nomeada (declarada com `::` e `lambda`).
fn call_named_function(
    ctx: &mut InterpCtx,
    callee: &Spanned<TypedExpr>,
    args: &[Value],
    env: &mut Env,
) -> Result<Value, InterpError> {
    // Se o callee é um Ident, procurar na tabela de funções
    if let TypedExprKind::Ident { name } = &callee.node.kind {
        // Clonar o nome para evitar borrow de callee enquanto emprestamos ctx
        let name = name.clone();
        // Procurar nas funções nomeadas do módulo.
        // Clonar Arc<TypedModule> para evitar borrow conflict.
        let module = ctx.module.clone();
        for func in &module.functions {
            if func.name == name {
                let clauses = func.clauses.clone();
                return call_typed_clauses(ctx, &clauses, args, env);
            }
        }
        // Procurar nas actions do módulo
        for action in &module.actions {
            if action.name == name {
                // Clonar a action para evitar borrow conflict
                let action_clone = action.clone();
                return call_action_body(ctx, &action_clone, args, env);
            }
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
    let n_captures = unsafe { std::ptr::read(ptr.add(2)) };

    // Ler captures
    let mut captures = Vec::new();
    for i in 0..n_captures {
        let cap_val = unsafe { std::ptr::read(ptr.add(3 + i as usize)) };
        captures.push(cap_val);
    }

    // Reconstruir cláusulas
    let clauses = unsafe { &*clauses_ptr };

    // Empilhar escopo e definir captures
    env.push_scope();
    // As captures são definidas com os nomes do Lambda.captures
    // Mas não temos acesso ao Lambda.captures aqui — os nomes das
    // captures estão nas cláusulas? Não, estão no TypedExprKind::Lambda.
    // Como resolver? O call_closure_value não tem acesso aos nomes.
    //
    // Alternativa: as captures são posicionais — a primeira cláusula
    // deve saber quais nomes usar. Mas TypedLambdaClause não tem
    // captures info. Precisamos de outra abordagem.
    //
    // Solução temporária: as captures são armazenadas como (nome, valor)
    // no struct da closure. Vamos armazenar pares (nome_ptr, valor).
    // Por enquanto, não definir captures (Fase 1 só precisa de lambdas
    // sem captures para fatorial/fib).

    let result = call_typed_clauses(ctx, clauses, args, env);
    env.pop_scope();
    result
}

/// Chama um conjunto de cláusulas tipadas com argumentos.
fn call_typed_clauses(
    ctx: &mut InterpCtx,
    clauses: &[TypedLambdaClause],
    args: &[Value],
    env: &mut Env,
) -> Result<Value, InterpError> {
    for clause in clauses {
        env.push_scope();

        // Ligar args como __param_{i} para que hooks (@log etc.) que
        // referenciam __param_0 possam acessar os argumentos originais.
        for (i, val) in args.iter().enumerate() {
            env.define(&format!("__param_{i}"), *val);
        }

        // Pattern match dos argumentos (liga variáveis do pattern)
        let mut all_match = true;
        for (i, pat) in clause.patterns.iter().enumerate() {
            if !match_pattern(pat, args[i], env) {
                all_match = false;
                break;
            }
        }

        if all_match {
            // with bindings (avaliados depois do pattern match, que liga
            // as variáveis do pattern; com_bindings podem referenciar essas variáveis)
            for wb in &clause.with_bindings {
                let v = eval(ctx, &wb.value, env)?;
                env.define(&wb.name, v);
            }

            // Se há guards, testar
            if !clause.guards.is_empty() {
                for guard in &clause.guards {
                    if let Some(ref cond) = guard.condition {
                        let cond_val = eval(ctx, cond, env)?;
                        if cond_val != 0 {
                            let result = eval(ctx, &guard.body, env);
                            env.pop_scope();
                            return result;
                        }
                    } else {
                        // otherwise
                        let result = eval(ctx, &guard.body, env);
                        env.pop_scope();
                        return result;
                    }
                }
                // Nenhum guard passou — tentar próxima cláusula
                env.pop_scope();
                continue;
            }
            // Sem guards — avaliar body
            let result = eval(ctx, &clause.body, env);
            env.pop_scope();
            return result;
        }
        env.pop_scope();
    }
    Err(InterpError::Runtime(format!(
        "nenhuma cláusula匹配: args={args:?}"
    )))
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
fn call_action(
    ctx: &mut InterpCtx,
    name: &str,
    args: &[Value],
    env: &mut Env,
) -> Result<Value, InterpError> {
    // Procurar a action no módulo
    // Clonar Arc<TypedModule> para evitar borrow conflict.
    let module = ctx.module.clone();
    for action in &module.actions {
        if action.name == name {
            let action_clone = action.clone();
            return call_action_body(ctx, &action_clone, args, env);
        }
    }
    Err(InterpError::Runtime(format!(
        "action não encontrada: {name}"
    )))
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
