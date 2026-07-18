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

mod instantiate;
mod instantiate_collections;
mod naming;

use std::collections::HashMap;

use kata_ast::Spanned;
use kata_core::dispatch::DispatchTable;
use kata_core::ty::Ty;
use kata_inference::{
    FusedStage, Substitutions, TypedAction, TypedExpr, TypedExprKind, TypedFunction,
    TypedLambdaClause, TypedModule, apply_subs, unify,
};

use instantiate::instantiate_function;
use naming::canonicalize_subs;

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
        let (new_overloads, new_functions) = monomorph_pass(&mut mono);
        if new_overloads.is_empty() {
            break;
        }
        // Registra as novas instâncias no DispatchTable e functions.
        for oi in &new_overloads {
            mono.dispatch_table.insert(oi.clone());
        }
        for func in new_functions {
            mono.functions.push(func);
        }
    }

    mono
}

/// Uma passada de monomorphização.
///
/// Coleta call sites, gera instâncias, rewrites callees.
/// Retorna as novas `TypedFunction` geradas (vazio se fixpoint).
fn monomorph_pass(
    mono: &mut MonoModule,
) -> (Vec<kata_core::dispatch::OverloadInfo>, Vec<TypedFunction>) {
    let mut instance_map: HashMap<(String, String), String> = HashMap::new();
    let mut new_overloads: Vec<kata_core::dispatch::OverloadInfo> = Vec::new();
    let mut new_functions: Vec<TypedFunction> = Vec::new();

    // Snapshot do DispatchTable e functions ANTES de mutar — evita borrow conflict.
    let dispatch_table = mono.dispatch_table.clone();
    let existing_names: std::collections::HashSet<String> =
        mono.functions.iter().map(|f| f.name.clone()).collect();
    let orig_functions: Vec<TypedFunction> = mono.functions.clone();

    let ctx = MonoCtx {
        dispatch_table: &dispatch_table,
        functions: &orig_functions,
        existing: &existing_names,
    };

    let mut acc = RewriteAcc {
        new_overloads: &mut new_overloads,
        new_functions: &mut new_functions,
    };

    // ── Funções nomeadas ──
    for func in &mut mono.functions {
        rewrite_function(func, &ctx, &mut instance_map, &mut acc);
    }

    // ── Actions ──
    for action in &mut mono.actions {
        rewrite_action(action, &ctx, &mut instance_map, &mut acc);
    }

    // ── pre_entry ──
    for expr in &mut mono.pre_entry {
        rewrite_typed_expr(expr, &ctx, &mut instance_map, &mut acc);
    }

    // ── entry ──
    rewrite_typed_expr(&mut mono.entry, &ctx, &mut instance_map, &mut acc);

    (new_overloads, new_functions)
}

/// Contexto imutável para a passada de monomorphização.
///
/// Snapshot do DispatchTable e functions antes de mutar, evitando
/// borrow conflicts entre iteração mutável e lookup imutável.
struct MonoCtx<'a> {
    dispatch_table: &'a DispatchTable,
    functions: &'a [TypedFunction],
    existing: &'a std::collections::HashSet<String>,
}

/// Acumulador mutable para a passada de monomorphização.
///
/// Centraliza as novas overloads e funções geradas, evitando
/// passar dois `&mut Vec` separados pelas chamadas recursivas.
struct RewriteAcc<'a> {
    new_overloads: &'a mut Vec<kata_core::dispatch::OverloadInfo>,
    new_functions: &'a mut Vec<TypedFunction>,
}

/// Rewrita call sites genéricos em uma `TypedFunction`.
fn rewrite_function(
    func: &mut TypedFunction,
    ctx: &MonoCtx,
    instance_map: &mut HashMap<(String, String), String>,
    acc: &mut RewriteAcc,
) {
    for clause in &mut func.clauses {
        rewrite_typed_expr(&mut clause.body, ctx, instance_map, acc);
        for guard in &mut clause.guards {
            if let Some(ref mut cond) = guard.condition {
                rewrite_typed_expr(cond, ctx, instance_map, acc);
            }
            rewrite_typed_expr(&mut guard.body, ctx, instance_map, acc);
        }
        for wb in &mut clause.with_bindings {
            rewrite_typed_expr(&mut wb.value, ctx, instance_map, acc);
        }
    }
}

/// Rewrita call sites genéricos em uma `TypedAction`.
fn rewrite_action(
    action: &mut TypedAction,
    ctx: &MonoCtx,
    instance_map: &mut HashMap<(String, String), String>,
    acc: &mut RewriteAcc,
) {
    for stmt in &mut action.body {
        rewrite_typed_expr(stmt, ctx, instance_map, acc);
    }
}

/// Rewrita call sites genéricos em um `Spanned<TypedExpr>`.
///
/// Se o expr é uma `Closure` cujo callee é `Ident(name)` e `name` tem
/// overload com `type_params` não-vazio, gera a instância e rewrites
/// o callee para o nome da instância.
fn rewrite_typed_expr(
    expr_span: &mut Spanned<TypedExpr>,
    ctx: &MonoCtx,
    instance_map: &mut HashMap<(String, String), String>,
    acc: &mut RewriteAcc,
) {
    let expr = &mut expr_span.node;

    match &mut expr.kind {
        TypedExprKind::Closure { callee, args, .. } => {
            // Primeiro recurse nos argumentos (podem ter call sites genéricos aninhados).
            for arg in args.iter_mut() {
                rewrite_typed_expr(arg, ctx, instance_map, acc);
            }

            // Depois verifica se este call site é genérico.
            #[allow(clippy::collapsible_if)]
            if let TypedExprKind::Ident { name } = &callee.node.kind {
                if let Some(overloads) = ctx.dispatch_table.get_overloads(name) {
                    // Procura overload genérica com mesma aridade dos args.
                    let arg_types: Vec<Ty> = args.iter().map(|a| a.node.ty.clone()).collect();
                    let generic_overload = overloads.iter().find(|oi| {
                        !oi.type_params.is_empty() && oi.params.len() == arg_types.len()
                    });

                    if let Some(oi) = generic_overload {
                        // Recomputa substitutions via unify.
                        let mut subs: Substitutions = HashMap::new();
                        if unify(&oi.params, &arg_types, &oi.type_params, &mut subs).is_ok() {
                            // Gera nome canônico da instância.
                            let subs_key = canonicalize_subs(&oi.type_params, &subs);
                            let instance_name = format!("{name}_{subs_key}");

                            // Verifica se a instância já existe.
                            if !ctx.existing.contains(&instance_name)
                                && !acc.new_overloads.iter().any(|o| o.name == instance_name)
                            {
                                // SEMPRE gera OverloadInfo (entrada no DispatchTable
                                // com tipos concretos). Isto cobre o caso de funções
                                // genéricas sem corpo (apenas Sig no DispatchTable,
                                // como `id :: T => T` sem cláusulas).
                                acc.new_overloads.push(kata_core::dispatch::OverloadInfo {
                                    name: instance_name.clone(),
                                    params: oi
                                        .params
                                        .iter()
                                        .map(|t| apply_subs(t, &subs))
                                        .collect(),
                                    ret: apply_subs(&oi.ret, &subs),
                                    ffi_symbol: oi.ffi_symbol.clone(),
                                    is_action: false,
                                    is_generic: false,
                                    is_constructor: false,
                                    associative_neutral: None,
                                    type_params: vec![],
                                    substitutions: Some(subs.clone()),
                                });

                                // SÓ gera TypedFunction se a função original tem corpo.
                                if let Some(orig_func) =
                                    ctx.functions.iter().find(|f| f.name == *name)
                                {
                                    let mono_func =
                                        instantiate_function(orig_func, &subs, &instance_name);
                                    acc.new_functions.push(mono_func);
                                }
                            }

                            // Atualiza o instance_map.
                            instance_map.insert((name.clone(), subs_key), instance_name.clone());

                            // Rewrite o callee para o nome da instância.
                            callee.node.kind = TypedExprKind::Ident {
                                name: instance_name,
                            };
                        }
                    }
                }
            }
        }

        // Recursão nos demais casos que contêm sub-expressões.
        TypedExprKind::TypeAscription { expr: inner, .. }
        | TypedExprKind::Grouping { inner }
        | TypedExprKind::Return(inner) => {
            rewrite_typed_expr(inner, ctx, instance_map, acc);
        }

        TypedExprKind::Tuple { elements }
        | TypedExprKind::StructConstruct {
            values: elements, ..
        } => {
            for elem in elements.iter_mut() {
                rewrite_typed_expr(elem, ctx, instance_map, acc);
            }
        }

        TypedExprKind::FieldAccess { expr: inner, .. }
        | TypedExprKind::IndexAccess { expr: inner, .. } => {
            rewrite_typed_expr(inner, ctx, instance_map, acc);
        }

        TypedExprKind::Let { value, .. }
        | TypedExprKind::Var { value, .. }
        | TypedExprKind::Reassign { value, .. } => {
            rewrite_typed_expr(value, ctx, instance_map, acc);
        }

        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses.iter_mut() {
                rewrite_lambda_clause(clause, ctx, instance_map, acc);
            }
        }

        TypedExprKind::Match { scrutinee, arms } => {
            rewrite_typed_expr(scrutinee, ctx, instance_map, acc);
            for arm in arms.iter_mut() {
                if let Some(ref mut guard) = arm.guard {
                    rewrite_typed_expr(guard, ctx, instance_map, acc);
                }
                rewrite_typed_expr(&mut arm.body, ctx, instance_map, acc);
            }
        }

        TypedExprKind::ActionCall { args, .. } => {
            rewrite_typed_expr(args, ctx, instance_map, acc);
        }

        TypedExprKind::Loop { body } => {
            for stmt in body.iter_mut() {
                rewrite_typed_expr(stmt, ctx, instance_map, acc);
            }
        }

        TypedExprKind::VariantConstruct { payload, .. } => {
            rewrite_typed_expr(payload, ctx, instance_map, acc);
        }

        // ── Coleções: recursão nos elementos ──
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements.iter_mut() {
                rewrite_typed_expr(el, ctx, instance_map, acc);
            }
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            rewrite_typed_expr(start, ctx, instance_map, acc);
            rewrite_typed_expr(step, ctx, instance_map, acc);
            rewrite_typed_expr(end, ctx, instance_map, acc);
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            rewrite_typed_expr(iterable, ctx, instance_map, acc);
            for stmt in body.iter_mut() {
                rewrite_typed_expr(stmt, ctx, instance_map, acc);
            }
        }
        TypedExprKind::In { item, collection } => {
            rewrite_typed_expr(item, ctx, instance_map, acc);
            rewrite_typed_expr(collection, ctx, instance_map, acc);
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
            rewrite_typed_expr(callback, ctx, instance_map, acc);
            rewrite_typed_expr(collection, ctx, instance_map, acc);
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            rewrite_typed_expr(callback, ctx, instance_map, acc);
            rewrite_typed_expr(initial, ctx, instance_map, acc);
            rewrite_typed_expr(collection, ctx, instance_map, acc);
        }
        // ── FusedStream: recursão ──
        TypedExprKind::FusedStream { stages, source, .. } => {
            rewrite_typed_expr(source, ctx, instance_map, acc);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                rewrite_typed_expr(cb, ctx, instance_map, acc);
            }
        }

        // Folhas — sem sub-expressões.
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue
        // ChannelCreate não tem sub-exprs (args consumidos pelo typeck).
        | TypedExprKind::ChannelCreate { .. } => {}
        // CSP — recursão.
        TypedExprKind::ChannelSend { channel, value } => {
            rewrite_typed_expr(channel, ctx, instance_map, acc);
            rewrite_typed_expr(value, ctx, instance_map, acc);
        }
        TypedExprKind::ChannelRecv { channel, .. } => {
            rewrite_typed_expr(channel, ctx, instance_map, acc);
        }
        TypedExprKind::Select { arms, timeout_ms, timeout_body } => {
            for arm in arms {
                rewrite_typed_expr(&mut arm.channel, ctx, instance_map, acc);
                rewrite_typed_expr(&mut arm.body, ctx, instance_map, acc);
            }
            if let Some(tm) = timeout_ms {
                rewrite_typed_expr(tm, ctx, instance_map, acc);
            }
            if let Some(tb) = timeout_body {
                rewrite_typed_expr(tb, ctx, instance_map, acc);
            }
        }
        TypedExprKind::Fork { args, .. } => {
            rewrite_typed_expr(args, ctx, instance_map, acc);
        }
    }
}

/// Rewrita call sites genéricos em uma `TypedLambdaClause`.
fn rewrite_lambda_clause(
    clause: &mut TypedLambdaClause,
    ctx: &MonoCtx,
    instance_map: &mut HashMap<(String, String), String>,
    acc: &mut RewriteAcc,
) {
    rewrite_typed_expr(&mut clause.body, ctx, instance_map, acc);
    for guard in &mut clause.guards {
        if let Some(ref mut cond) = guard.condition {
            rewrite_typed_expr(cond, ctx, instance_map, acc);
        }
        rewrite_typed_expr(&mut guard.body, ctx, instance_map, acc);
    }
    for wb in &mut clause.with_bindings {
        rewrite_typed_expr(&mut wb.value, ctx, instance_map, acc);
    }
}
