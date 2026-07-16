//! Fase 6: Monomorphização — especializa call sites genéricos em funções concretas.
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

use std::collections::HashMap;

use kata_ast::Spanned;
use kata_core::dispatch::DispatchTable;
use kata_core::ty::Ty;
use kata_inference::{
    Substitutions, TypedAction, TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause,
    TypedModule, TypedPattern, TypedWithBinding, apply_subs, unify,
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

        // Folhas — sem sub-expressões.
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue => {}
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

// ── Instanciação ─────────────────────────────────────────────

/// Gera uma instância monomorfizada de uma `TypedFunction`.
///
/// Substitui todos os `Ty::Var("T")` pelos tipos concretos em `subs`
/// nos param_types, ret_ty, e no corpo de cada cláusula.
fn instantiate_function(
    orig: &TypedFunction,
    subs: &Substitutions,
    instance_name: &str,
) -> TypedFunction {
    let param_types: Vec<Ty> = orig
        .param_types
        .iter()
        .map(|t| apply_subs(t, subs))
        .collect();
    let ret_ty = apply_subs(&orig.ret_ty, subs);
    let clauses: Vec<TypedLambdaClause> = orig
        .clauses
        .iter()
        .map(|c| instantiate_clause(c, subs))
        .collect();

    TypedFunction {
        name: instance_name.to_string(),
        param_types,
        ret_ty,
        clauses,
    }
}

/// Instancia uma cláusula — substitui Ty::Var nos padrões e corpo.
fn instantiate_clause(clause: &TypedLambdaClause, subs: &Substitutions) -> TypedLambdaClause {
    TypedLambdaClause {
        patterns: clause
            .patterns
            .iter()
            .map(|p| Spanned::new(instantiate_pattern(&p.node, subs), p.span))
            .collect(),
        body: Spanned::new(
            instantiate_typed_expr(&clause.body.node, subs),
            clause.body.span,
        ),
        guards: clause
            .guards
            .iter()
            .map(|g| instantiate_guard(g, subs))
            .collect(),
        with_bindings: clause
            .with_bindings
            .iter()
            .map(|wb| TypedWithBinding {
                name: wb.name.clone(),
                value: Spanned::new(instantiate_typed_expr(&wb.value.node, subs), wb.value.span),
            })
            .collect(),
    }
}

/// Instancia um guard.
fn instantiate_guard(
    guard: &kata_inference::TypedGuardClause,
    subs: &Substitutions,
) -> kata_inference::TypedGuardClause {
    kata_inference::TypedGuardClause {
        condition: guard
            .condition
            .as_ref()
            .map(|c| Spanned::new(instantiate_typed_expr(&c.node, subs), c.span)),
        body: Spanned::new(
            instantiate_typed_expr(&guard.body.node, subs),
            guard.body.span,
        ),
    }
}

/// Instancia um `TypedPattern` — substitui Ty::Var no tipo dos bindings.
fn instantiate_pattern(pattern: &TypedPattern, subs: &Substitutions) -> TypedPattern {
    match pattern {
        TypedPattern::Ident { name, ty } => TypedPattern::Ident {
            name: name.clone(),
            ty: apply_subs(ty, subs),
        },
        TypedPattern::Wildcard => TypedPattern::Wildcard,
        TypedPattern::Literal { value } => TypedPattern::Literal {
            value: Spanned::new(instantiate_typed_expr(&value.node, subs), value.span),
        },
        TypedPattern::Variant {
            enum_name,
            variant,
            sub_patterns,
            tag,
        } => TypedPattern::Variant {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            sub_patterns: sub_patterns.as_ref().map(|sps| {
                sps.iter()
                    .map(|sp| Spanned::new(instantiate_pattern(&sp.node, subs), sp.span))
                    .collect()
            }),
            tag: *tag,
        },
        TypedPattern::Tuple { elements } => TypedPattern::Tuple {
            elements: elements
                .iter()
                .map(|e| Spanned::new(instantiate_pattern(&e.node, subs), e.span))
                .collect(),
        },
        TypedPattern::Cons { head, tail } => TypedPattern::Cons {
            head: Box::new(Spanned::new(
                instantiate_pattern(&head.node, subs),
                head.span,
            )),
            tail: Box::new(Spanned::new(
                instantiate_pattern(&tail.node, subs),
                tail.span,
            )),
        },
    }
}

/// Instancia um `TypedExpr` — substitui Ty::Var no tipo do nó e recurse nos filhos.
fn instantiate_typed_expr(expr: &TypedExpr, subs: &Substitutions) -> TypedExpr {
    TypedExpr {
        span: expr.span,
        ty: apply_subs(&expr.ty, subs),
        tail_pos: expr.tail_pos,
        escape: expr.escape,
        effect: expr.effect,
        kind: instantiate_kind(&expr.kind, subs),
    }
}

/// Instancia um `TypedExprKind` — recursão nos filhos.
fn instantiate_kind(kind: &TypedExprKind, subs: &Substitutions) -> TypedExprKind {
    match kind {
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => TypedExprKind::Closure {
            callee: Box::new(Spanned::new(
                instantiate_typed_expr(&callee.node, subs),
                callee.span,
            )),
            args: args
                .iter()
                .map(|a| Spanned::new(instantiate_typed_expr(&a.node, subs), a.span))
                .collect(),
            ffi_symbol: ffi_symbol.clone(),
        },

        TypedExprKind::TypeAscription {
            expr: inner,
            target_ty,
        } => TypedExprKind::TypeAscription {
            expr: Box::new(Spanned::new(
                instantiate_typed_expr(&inner.node, subs),
                inner.span,
            )),
            target_ty: apply_subs(target_ty, subs),
        },

        TypedExprKind::Grouping { inner } => TypedExprKind::Grouping {
            inner: Box::new(Spanned::new(
                instantiate_typed_expr(&inner.node, subs),
                inner.span,
            )),
        },

        TypedExprKind::Tuple { elements } => TypedExprKind::Tuple {
            elements: elements
                .iter()
                .map(|e| Spanned::new(instantiate_typed_expr(&e.node, subs), e.span))
                .collect(),
        },

        TypedExprKind::StructConstruct {
            struct_name,
            values,
        } => TypedExprKind::StructConstruct {
            struct_name: struct_name.clone(),
            values: values
                .iter()
                .map(|v| Spanned::new(instantiate_typed_expr(&v.node, subs), v.span))
                .collect(),
        },

        TypedExprKind::FieldAccess {
            expr: inner,
            struct_name,
            field_name,
            field_index,
        } => TypedExprKind::FieldAccess {
            expr: Box::new(Spanned::new(
                instantiate_typed_expr(&inner.node, subs),
                inner.span,
            )),
            struct_name: struct_name.clone(),
            field_name: field_name.clone(),
            field_index: *field_index,
        },

        TypedExprKind::IndexAccess {
            expr: inner,
            index,
            element_index,
        } => TypedExprKind::IndexAccess {
            expr: Box::new(Spanned::new(
                instantiate_typed_expr(&inner.node, subs),
                inner.span,
            )),
            index: *index,
            element_index: *element_index,
        },

        TypedExprKind::Let { name, value } => TypedExprKind::Let {
            name: name.clone(),
            value: Box::new(Spanned::new(
                instantiate_typed_expr(&value.node, subs),
                value.span,
            )),
        },

        TypedExprKind::Lambda {
            func_name,
            param_types,
            ret_ty,
            clauses,
            captures,
        } => TypedExprKind::Lambda {
            func_name: func_name.clone(),
            param_types: param_types.iter().map(|t| apply_subs(t, subs)).collect(),
            ret_ty: apply_subs(ret_ty, subs),
            clauses: clauses
                .iter()
                .map(|c| instantiate_clause(c, subs))
                .collect(),
            captures: captures
                .iter()
                .map(|c| kata_inference::CaptureInfo {
                    name: c.name.clone(),
                    ty: apply_subs(&c.ty, subs),
                })
                .collect(),
        },

        TypedExprKind::Match { scrutinee, arms } => TypedExprKind::Match {
            scrutinee: Box::new(Spanned::new(
                instantiate_typed_expr(&scrutinee.node, subs),
                scrutinee.span,
            )),
            arms: arms
                .iter()
                .map(|arm| kata_inference::TypedMatchArm {
                    pattern: arm
                        .pattern
                        .as_ref()
                        .map(|p| Spanned::new(instantiate_pattern(&p.node, subs), p.span)),
                    guard: arm
                        .guard
                        .as_ref()
                        .map(|g| Spanned::new(instantiate_typed_expr(&g.node, subs), g.span)),
                    body: Spanned::new(instantiate_typed_expr(&arm.body.node, subs), arm.body.span),
                })
                .collect(),
        },

        TypedExprKind::VariantQual {
            enum_name,
            variant,
            tag,
        } => TypedExprKind::VariantQual {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            tag: *tag,
        },

        TypedExprKind::VariantConstruct {
            enum_name,
            variant,
            payload,
            tag,
        } => TypedExprKind::VariantConstruct {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            payload: Box::new(Spanned::new(
                instantiate_typed_expr(&payload.node, subs),
                payload.span,
            )),
            tag: *tag,
        },

        TypedExprKind::Var { name, value } => TypedExprKind::Var {
            name: name.clone(),
            value: Box::new(Spanned::new(
                instantiate_typed_expr(&value.node, subs),
                value.span,
            )),
        },

        TypedExprKind::Reassign { name, value } => TypedExprKind::Reassign {
            name: name.clone(),
            value: Box::new(Spanned::new(
                instantiate_typed_expr(&value.node, subs),
                value.span,
            )),
        },

        TypedExprKind::Return(inner) => TypedExprKind::Return(Box::new(Spanned::new(
            instantiate_typed_expr(&inner.node, subs),
            inner.span,
        ))),

        TypedExprKind::ActionCall {
            callee,
            args,
            caller_arena,
            ffi_symbol,
        } => TypedExprKind::ActionCall {
            callee: callee.clone(),
            args: Box::new(Spanned::new(
                instantiate_typed_expr(&args.node, subs),
                args.span,
            )),
            caller_arena: *caller_arena,
            ffi_symbol: ffi_symbol.clone(),
        },

        TypedExprKind::Loop { body } => TypedExprKind::Loop {
            body: body
                .iter()
                .map(|s| Spanned::new(instantiate_typed_expr(&s.node, subs), s.span))
                .collect(),
        },

        // Folhas — sem sub-expressões, sem Ty::Var.
        TypedExprKind::IntLit { text } => TypedExprKind::IntLit { text: text.clone() },
        TypedExprKind::FloatLit { text } => TypedExprKind::FloatLit { text: text.clone() },
        TypedExprKind::TextLit { text } => TypedExprKind::TextLit { text: text.clone() },
        TypedExprKind::Unit => TypedExprKind::Unit,
        TypedExprKind::Ident { name } => TypedExprKind::Ident { name: name.clone() },
        TypedExprKind::Break => TypedExprKind::Break,
        TypedExprKind::Continue => TypedExprKind::Continue,
    }
}

// ── Naming ────────────────────────────────────────────────────

/// Gera uma string canônica para um mapa de substitutions.
///
/// Ordena por nome do type param para que `T=Int, E=Text` e `E=Text, T=Int`
/// produzam a mesma chave.
fn canonicalize_subs(type_params: &[String], subs: &Substitutions) -> String {
    let mut parts: Vec<String> = Vec::new();
    for param in type_params {
        if let Some(ty) = subs.get(param) {
            parts.push(format!("{param}_{}", ty_to_string(ty)));
        }
    }
    parts.join("_")
}

/// Converte um `Ty` para string canônica usada no nome da instância.
fn ty_to_string(ty: &Ty) -> String {
    match ty {
        Ty::Prim(p) => format!("{p:?}"),
        Ty::Var(name) => name.clone(),
        Ty::Generic(name, args) => {
            let args_str = args.iter().map(ty_to_string).collect::<Vec<_>>().join("_");
            format!("{name}_{args_str}")
        }
        Ty::Sum(name) => name.clone(),
        Ty::Function(params, ret) => {
            let p = params
                .iter()
                .map(ty_to_string)
                .collect::<Vec<_>>()
                .join("_");
            format!("Fn_{p}_{}", ty_to_string(ret))
        }
        Ty::Tuple(elems) => {
            let e = elems.iter().map(ty_to_string).collect::<Vec<_>>().join("_");
            format!("Tup_{e}")
        }
        Ty::Unit => "Unit".to_string(),
        Ty::InferVar(_) => "Inf".to_string(),
        Ty::Interface(name) => format!("Iface_{name}"),
        _ => format!("{ty:?}"),
    }
}
