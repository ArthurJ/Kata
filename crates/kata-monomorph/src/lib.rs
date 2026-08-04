//! Monomorphização — especializa call sites genéricos em funções concretas.
//!
//! Recebe `TypedModule` (TAST com tipos genéricos) → produz `MonoModule`
//! (TAST com tipos concretos). Cada call site genérico é substituído por
//! uma chamada para uma função especializada.
//!
//! ## Algoritmo
//!
//! 1. Coletar todos os call sites genéricos na TAST (funções com
//!    `type_params` não-vazio no DispatchTable)
//! 2. Para cada call site, recomputar as substitutions concretas via
//!    `unify` (reutilizando a função de kata-inference)
//! 3. Para cada combinação única de (função, substitutions), gerar uma
//!    instância monomorfizada:
//!    - Nome único: `original_name_T_Int` (ou hash se complexo)
//!    - Substituir todos os `Ty::Var("T")` pelo tipo concreto no body
//!    - Registrar como nova `TypedFunction` no module
//! 4. Substituir o call site genérico por uma chamada para a instância
//! 5. Repetir até fixpoint (instâncias monomorfizadas podem ter novos
//!    call sites genéricos)
//!
//! ## Por que recomputar substitutions?
//!
//! O inference (`infer_apply`) faz `unify` mas não armazena as substitutions
//! na TAST — ele só aplica `apply_subs` no tipo de retorno. O monomorphizador
//! recompute as substitutions comparando os tipos dos params (que são
//! `Ty::Var("T")` na função genérica original) com os tipos dos argumentos
//! (que são concretos). Isto reutiliza `unify` de kata-inference sem
//! duplicação.

mod fallback;
mod instantiate;
mod instantiate_collections;
mod naming;
mod overload_resolution;
mod tuple_show;

use kata_ast::Spanned;
use kata_core::dispatch::DispatchTable;
use kata_core::ty::Ty;
use kata_inference::{
    FusedStage, TypedAction, TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause,
    TypedModule, TypedSelectArm,
};

use overload_resolution::{
    instantiate_generic_action_call, instantiate_generic_closure, resolve_erased_ffi_symbol,
};

/// Módulo monomorfizado — TAST com todos os tipos concretos.
///
/// Se o módulo não tem generics, é idêntico ao `TypedModule` de entrada.
/// O codegen consome isto em vez de `TypedModule`.
///
/// Implementa `Deref<Target = TypedModule>` para que o codegen e o optimizer
/// possam operar via `&TypedModule` sem mudanças.
#[derive(Debug, Clone)]
pub struct MonoModule {
    pub inner: TypedModule,
}

impl From<TypedModule> for MonoModule {
    fn from(tm: TypedModule) -> Self {
        MonoModule { inner: tm }
    }
}

impl std::ops::Deref for MonoModule {
    type Target = TypedModule;
    fn deref(&self) -> &TypedModule {
        &self.inner
    }
}

impl std::ops::DerefMut for MonoModule {
    fn deref_mut(&mut self) -> &mut TypedModule {
        &mut self.inner
    }
}

/// Monomorphiza um `TypedModule`.
///
/// Percorre a TAST procurando call sites genéricos, gera instâncias
/// concretas, e rewrites os callees. Repete até fixpoint.
pub fn monomorphize(typed: TypedModule) -> MonoModule {
    let mut mono: MonoModule = typed.into();

    // Fixpoint: a cada iteração, coleta call sites genéricos, gera
    // instâncias, e rewrites. Se nenhuma nova instância foi gerada, para.
    loop {
        let (new_overloads, new_functions, new_actions) = monomorph_pass(&mut mono);
        if new_overloads.is_empty() {
            break;
        }
        // Registra as novas instâncias no DispatchTable, functions e actions.
        for oi in &new_overloads {
            mono.dispatch_table.insert(oi.clone());
        }
        for func in new_functions {
            mono.functions.push(func);
        }
        for action in new_actions {
            mono.actions.push(action);
        }
    }

    // Passada final: aplica fallback gracioso a Closures com ffi_symbol: None
    // cujo arg_type é Ty::Var(_) não resolvido (ex: braço Err de show_Result
    // quando só Result::Ok aparece). O braço nunca executa em runtime, mas o
    // codegen precisa de um nó válido — substitui por TextLit("?").
    fallback::fallback_unresolved_show(&mut mono);

    mono
}

/// Uma passada de monomorphização.
///
/// Coleta call sites, gera instâncias, rewrites callees.
/// Retorna as novas `TypedFunction` geradas (vazio se fixpoint).
fn monomorph_pass(
    mono: &mut MonoModule,
) -> (
    Vec<kata_core::dispatch::OverloadInfo>,
    Vec<TypedFunction>,
    Vec<TypedAction>,
) {
    let mut new_overloads: Vec<kata_core::dispatch::OverloadInfo> = Vec::new();
    let mut new_functions: Vec<TypedFunction> = Vec::new();
    let mut new_actions: Vec<TypedAction> = Vec::new();

    // Snapshot do DispatchTable, functions e actions ANTES de mutar — evita borrow conflict.
    let dispatch_table = mono.dispatch_table.clone();
    let existing_names: std::collections::HashSet<String> = mono
        .functions
        .iter()
        .map(|f| f.name.clone())
        .chain(mono.actions.iter().map(|a| a.name.clone()))
        .collect();
    let orig_functions: Vec<TypedFunction> = mono.functions.clone();
    let orig_actions: Vec<TypedAction> = mono.actions.clone();

    let ctx = MonoCtx {
        dispatch_table: &dispatch_table,
        functions: &orig_functions,
        actions: &orig_actions,
        existing: &existing_names,
    };

    let mut acc = RewriteAcc {
        new_overloads: &mut new_overloads,
        new_functions: &mut new_functions,
        new_actions: &mut new_actions,
    };

    // ── Funções nomeadas ──
    for func in &mut mono.functions {
        rewrite_function(func, &ctx, &mut acc);
    }

    // ── Actions ──
    for action in &mut mono.actions {
        rewrite_action(action, &ctx, &mut acc);
    }

    // ── pre_entry ──
    for expr in &mut mono.pre_entry {
        rewrite_typed_expr(expr, &ctx, &mut acc);
    }

    // ── entry ──
    rewrite_typed_expr(&mut mono.entry, &ctx, &mut acc);

    (new_overloads, new_functions, new_actions)
}

/// Contexto imutável para a passada de monomorphização.
///
/// Snapshot do DispatchTable e functions antes de mutar, evitando
/// borrow conflicts entre iteração mutável e lookup imutável.
pub(crate) struct MonoCtx<'a> {
    dispatch_table: &'a DispatchTable,
    functions: &'a [TypedFunction],
    actions: &'a [TypedAction],
    existing: &'a std::collections::HashSet<String>,
}

/// Acumulador mutable para a passada de monomorphização.
///
/// Centraliza as novas overloads, funções e actions geradas, evitando
/// passar três `&mut Vec` separados pelas chamadas recursivas.
pub(crate) struct RewriteAcc<'a> {
    new_overloads: &'a mut Vec<kata_core::dispatch::OverloadInfo>,
    new_functions: &'a mut Vec<TypedFunction>,
    new_actions: &'a mut Vec<TypedAction>,
}

/// Rewrita call sites genéricos em uma `TypedFunction`.
fn rewrite_function(func: &mut TypedFunction, ctx: &MonoCtx, acc: &mut RewriteAcc) {
    for clause in &mut func.clauses {
        rewrite_typed_expr(&mut clause.body, ctx, acc);
        for guard in &mut clause.guards {
            if let Some(ref mut cond) = guard.condition {
                rewrite_typed_expr(cond, ctx, acc);
            }
            rewrite_typed_expr(&mut guard.body, ctx, acc);
        }
        for wb in &mut clause.with_bindings {
            rewrite_typed_expr(&mut wb.value, ctx, acc);
        }
    }
}

/// Rewrita call sites genéricos em uma `TypedAction`.
fn rewrite_action(action: &mut TypedAction, ctx: &MonoCtx, acc: &mut RewriteAcc) {
    for stmt in &mut action.body {
        rewrite_typed_expr(stmt, ctx, acc);
    }
}

/// Rewrita call sites genéricos em um `Spanned<TypedExpr>`.
///
/// Se o expr é uma `Closure` cujo callee é `Ident(name)` e `name` tem
/// overload com `type_params` não-vazio, gera a instância e rewrites
/// o callee para o nome da instância.
fn rewrite_typed_expr(expr_span: &mut Spanned<TypedExpr>, ctx: &MonoCtx, acc: &mut RewriteAcc) {
    let expr = &mut expr_span.node;

    match &mut expr.kind {
        TypedExprKind::Closure { callee, args, ffi_symbol } => {
            // Primeiro recurse nos argumentos (podem ter call sites genéricos aninhados).
            for arg in args.iter_mut() {
                rewrite_typed_expr(arg, ctx, acc);
            }

            // Depois verifica se este call site é genérico ou precisa de
            // resolução de ffi_symbol (Layer 5).
            if let TypedExprKind::Ident { name } = &callee.node.kind {
                let name = name.clone();
                let instantiated =
                    instantiate_generic_closure(callee, args, ffi_symbol, &name, ctx, acc);
                if !instantiated {
                    resolve_erased_ffi_symbol(&name, args, ffi_symbol, ctx);
                }

                // Layer 6: show de Tuple sem overload concreto.
                // Se o ffi_symbol ainda é None e o callee é `show` com arg
                // Ty::Tuple, substitui a Closure inteira pelo body inline
                // (string_concat de show de cada elemento via FieldAccess).
                if ffi_symbol.is_none() && name == "show" && args.len() == 1
                    && let Ty::Tuple(_) = &args[0].node.ty
                {
                    let replacement = crate::tuple_show::rewrite_show_tuple_call(
                        &args[0],
                    );
                    *expr_span = replacement;
                }
            }
        }

        // Recursão nos demais casos que contêm sub-expressões.
        TypedExprKind::TypeAscription { expr: inner, .. }
        | TypedExprKind::Grouping { inner }
        | TypedExprKind::Return(inner) => {
            rewrite_typed_expr(inner, ctx, acc);
        }

        TypedExprKind::Tuple { elements }
        | TypedExprKind::StructConstruct {
            values: elements, ..
        } => {
            for elem in elements.iter_mut() {
                rewrite_typed_expr(elem, ctx, acc);
            }
        }

        TypedExprKind::FieldAccess { expr: inner, .. }
        | TypedExprKind::IndexAccess { expr: inner, .. } => {
            rewrite_typed_expr(inner, ctx, acc);
        }

        TypedExprKind::Let { value, .. }
        | TypedExprKind::LetDestruct { value, .. }
        | TypedExprKind::Var { value, .. }
        | TypedExprKind::Reassign { value, .. } => {
            rewrite_typed_expr(value, ctx, acc);
        }

        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses.iter_mut() {
                rewrite_lambda_clause(clause, ctx, acc);
            }
        }

        TypedExprKind::Match { scrutinee, arms } => {
            rewrite_typed_expr(scrutinee, ctx, acc);
            for arm in arms.iter_mut() {
                if let Some(ref mut guard) = arm.guard {
                    rewrite_typed_expr(guard, ctx, acc);
                }
                rewrite_typed_expr(&mut arm.body, ctx, acc);
            }
        }

        TypedExprKind::ActionCall {
            callee,
            args,
            caller_arena: _,
            ffi_symbol,
            indirect_callee,
        } => {
            // Primeiro recurse nos argumentos (podem ter call sites genéricos aninhados).
            rewrite_typed_expr(args, ctx, acc);

            // Recursão no indirect_callee (se presente — call indireto).
            if let Some(ic) = indirect_callee {
                rewrite_typed_expr(ic, ctx, acc);
            }

            // Depois verifica se este ActionCall é genérico.
            // FFI builtins (ffi_symbol = Some) não são instanciados.
            if ffi_symbol.is_none() {
                instantiate_generic_action_call(callee, args, ctx, acc);
            }
        }

        TypedExprKind::TypeOf { expr } => {
            rewrite_typed_expr(expr, ctx, acc);
        }

        TypedExprKind::Loop { body } => {
            for stmt in body.iter_mut() {
                rewrite_typed_expr(stmt, ctx, acc);
            }
        }

        TypedExprKind::VariantConstruct { payload, .. } => {
            rewrite_typed_expr(payload, ctx, acc);
        }

        // ── Coleções: recursão nos elementos ──
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements.iter_mut() {
                rewrite_typed_expr(el, ctx, acc);
            }
        }
        TypedExprKind::DictLit { entries, .. } => {
            for (key, val) in entries.iter_mut() {
                rewrite_typed_expr(key, ctx, acc);
                rewrite_typed_expr(val, ctx, acc);
            }
        }
        TypedExprKind::SetLit { elements, .. } => {
            for el in elements.iter_mut() {
                rewrite_typed_expr(el, ctx, acc);
            }
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            rewrite_typed_expr(start, ctx, acc);
            rewrite_typed_expr(step, ctx, acc);
            rewrite_typed_expr(end, ctx, acc);
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            rewrite_typed_expr(iterable, ctx, acc);
            for stmt in body.iter_mut() {
                rewrite_typed_expr(stmt, ctx, acc);
            }
        }
        TypedExprKind::In { item, collection } => {
            rewrite_typed_expr(item, ctx, acc);
            rewrite_typed_expr(collection, ctx, acc);
        }

        // ── map/filter/fold: recursão ──
        TypedExprKind::Map {
            callback,
            collection,
            ..
        }
        | TypedExprKind::Filter {
            callback,
            collection,
            ..
        } => {
            rewrite_typed_expr(callback, ctx, acc);
            rewrite_typed_expr(collection, ctx, acc);
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            rewrite_typed_expr(callback, ctx, acc);
            rewrite_typed_expr(initial, ctx, acc);
            rewrite_typed_expr(collection, ctx, acc);
        }
        // ── FusedStream: recursão ──
        TypedExprKind::FusedStream { stages, source, .. } => {
            rewrite_typed_expr(source, ctx, acc);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                rewrite_typed_expr(cb, ctx, acc);
            }
        }

        // Folhas — sem sub-expressões.
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::BytesLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue
        // ChannelCreate não tem sub-exprs (args consumidos pelo typeck).
        | TypedExprKind::ChannelCreate { .. } => {}
        // Comptime — recursão no inner expr.
        TypedExprKind::Comptime { expr } => {
            rewrite_typed_expr(expr, ctx, acc);
        }
        // HeapSnapshot — folha.
        TypedExprKind::HeapSnapshot { .. } => {}
        // ReceiverFactoryCall: o factory é sub-expr (Ident do rxf).
        TypedExprKind::ReceiverFactoryCall { factory, .. } => {
            rewrite_typed_expr(factory, ctx, acc);
        }
        // CSP — recursão.
        TypedExprKind::ChannelSend { channel, value } => {
            rewrite_typed_expr(channel, ctx, acc);
            rewrite_typed_expr(value, ctx, acc);
        }
        TypedExprKind::ChannelRecv { channel, .. } => {
            rewrite_typed_expr(channel, ctx, acc);
        }
        TypedExprKind::Select { arms, timeout_ms, timeout_body } => {
            for arm in arms {
                match arm {
                    TypedSelectArm::Channel { channel, body, .. } => {
                        rewrite_typed_expr(channel, ctx, acc);
                        rewrite_typed_expr(body, ctx, acc);
                    }
                    TypedSelectArm::IoRead {
                        handle_expr,
                        chunk_size_expr,
                        body,
                        ..
                    } => {
                        rewrite_typed_expr(handle_expr, ctx, acc);
                        rewrite_typed_expr(chunk_size_expr, ctx, acc);
                        rewrite_typed_expr(body, ctx, acc);
                    }
                }
            }
            if let Some(tm) = timeout_ms {
                rewrite_typed_expr(tm, ctx, acc);
            }
            if let Some(tb) = timeout_body {
                rewrite_typed_expr(tb, ctx, acc);
            }
        }
        TypedExprKind::Fork { action_expr, args, .. } => {
            rewrite_typed_expr(action_expr, ctx, acc);
            rewrite_typed_expr(args, ctx, acc);
        }
        TypedExprKind::Spawn { action_expr, args, .. } => {
            rewrite_typed_expr(action_expr, ctx, acc);
            rewrite_typed_expr(args, ctx, acc);
        }
        // Block — recursão em cada stmt.
        TypedExprKind::Block { stmts } => {
            for stmt in stmts {
                rewrite_typed_expr(stmt, ctx, acc);
            }
        }
    }
}

/// Rewrita call sites genéricos em uma `TypedLambdaClause`.
fn rewrite_lambda_clause(clause: &mut TypedLambdaClause, ctx: &MonoCtx, acc: &mut RewriteAcc) {
    rewrite_typed_expr(&mut clause.body, ctx, acc);
    for guard in &mut clause.guards {
        if let Some(ref mut cond) = guard.condition {
            rewrite_typed_expr(cond, ctx, acc);
        }
        rewrite_typed_expr(&mut guard.body, ctx, acc);
    }
    for wb in &mut clause.with_bindings {
        rewrite_typed_expr(&mut wb.value, ctx, acc);
    }
}
