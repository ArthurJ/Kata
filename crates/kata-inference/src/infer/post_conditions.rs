//! Post-condições inter-procedurais — Nível 2 do refinement propagation.
//!
//! Quando uma função tem guards que decidem entre variants de um enum
//! (ex: `Result::Ok` vs `Result::Err`), o caller que faz
//! `match (f a b): Ok n: ...` aprende a condição que fez aquele variant
//! ser produzido.
//!
//! Ver `docs/PRDs/PRD-refinement-propagation.md` §9.

use std::collections::HashMap;

use kata_ast::{GuardClause, Pattern, Spanned};
use kata_core::ty::{Ty, TypeEnv};
use kata_resolution::FunctionDef;

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr, infer_expr_hinted};
use super::helpers::{check_patterns, process_with_bindings};

/// Uma post-condição: condição que produz um variant específico de um enum.
#[derive(Debug, Clone)]
pub(crate) struct PostCondition {
    /// Enum ao qual o variant pertence (ex: "Result", "Optional").
    pub enum_name: String,
    /// Variante do enum (ex: "Ok", "Err", "Some", "None").
    pub variant: String,
    /// Condição (sobre os params da função) que produz este variant.
    /// Já tipada — pronta para substituição parâmetro→argumento.
    pub condition: TypedExpr,
    /// Nomes dos parâmetros da função, na ordem posicional.
    /// Extraídos dos patterns da primeira cláusula lambda.
    pub param_names: Vec<String>,
    /// Tipos dos parâmetros (para desambiguar overloads no consumo).
    pub param_types: Vec<Ty>,
    /// Expressão do payload do variant (sobre os params da função).
    /// `None` para variantes unitárias ou quando guards que produzem o
    /// mesmo variant têm payloads divergentes.
    /// Usado para conectar o binding do pattern ao valor de retorno.
    pub payload: Option<TypedExpr>,
}

/// Tabela de post-condições por nome de função.
#[derive(Debug, Clone, Default)]
pub(crate) struct PostCondTable {
    map: HashMap<String, Vec<PostCondition>>,
}

impl PostCondTable {
    /// Consulta post-condições de uma função pelo nome.
    pub(crate) fn get(&self, func_name: &str) -> Option<&[PostCondition]> {
        self.map.get(func_name).map(|v| v.as_slice())
    }

    /// True se a tabela está vazia.
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Extrai post-condições de funções com guards que produzem variants de enum.
///
/// Percorre cada `FunctionDef` cujo retorno é um enum. Para cada guard,
/// classifica o body em `(enum_name, variant)` e deriva a condição que
/// produz cada variant.
///
/// Conservador: se qualquer body não é classificável ou a tipagem falha,
/// a função inteira é pulada (nenhuma post-condição registrada).
pub(crate) fn extract_post_conditions(
    functions: &[FunctionDef],
    ctx: &InferCtx,
    module_type_env: &TypeEnv,
) -> PostCondTable {
    let mut map: HashMap<String, Vec<PostCondition>> = HashMap::new();

    for func_def in functions {
        if let Some(post_conds) = extract_for_function(func_def, ctx, module_type_env)
            && !post_conds.is_empty()
        {
            map.entry(func_def.name.clone())
                .or_default()
                .extend(post_conds);
        }
    }

    PostCondTable { map }
}

/// Extrai post-condições de uma única função.
///
/// Retorna `None` (skip) se:
/// - O retorno não é enum (`Ty::Sum` ou `Ty::Generic`)
/// - Não há cláusulas
/// - Algum body não é diretamente `VariantConstruct`/`VariantQual`
/// - Tipagem de guard ou body falha
fn extract_for_function(
    func_def: &FunctionDef,
    ctx: &InferCtx,
    module_type_env: &TypeEnv,
) -> Option<Vec<PostCondition>> {
    // 1. Verifica se o retorno é um enum.
    let _enum_name = return_type_enum_name(&func_def.return_type)?;

    // 2. Extrai param_names da primeira cláusula.
    let first_clause = func_def.clauses.first()?;
    let param_names = extract_param_names(&first_clause.node.patterns);

    // 3. Percorre todas as cláusulas, coletando (condition, variant, payload) entries.
    // condition = None para otherwise sem guards anteriores (True — não adiciona info).
    // payload = expressão do payload do variant (None para variantes unitárias).
    let mut entries: Vec<(Option<TypedExpr>, String, String, Option<TypedExpr>)> = Vec::new();

    for clause in &func_def.clauses {
        let clause_inner = &clause.node;

        // Cria escopo filho para a cláusula, com acesso aos tipos do módulo.
        let mut clause_env = TypeEnv::with_parent(module_type_env.clone());

        // Desugar guards e with_bindings (elimina Pipe/Hole antes do typeck).
        let desugared_guards: Vec<GuardClause> = clause_inner
            .guards
            .iter()
            .map(|g| GuardClause {
                condition: g.condition.as_ref().map(crate::desugar::desugar),
                body: crate::desugar::desugar(&g.body),
            })
            .collect();
        let desugared_with_bindings: Vec<kata_ast::WithBinding> = clause_inner
            .with_bindings
            .iter()
            .map(|w| kata_ast::WithBinding {
                name: w.name.clone(),
                value: crate::desugar::desugar(&w.value),
            })
            .collect();

        // Check patterns para definir bindings no escopo da cláusula.
        check_patterns(
            &clause_inner.patterns,
            &func_def.param_types,
            ctx.enum_registry,
            &mut clause_env,
            ctx.interface_registry,
            ctx.struct_registry,
            ctx.refined_decls,
        )
        .ok()?;

        // Process with bindings (pode afetar tipagem dos guards).
        process_with_bindings(&desugared_with_bindings, &mut clause_env, ctx).ok()?;

        // Se não há guards, classificar o body diretamente.
        if desugared_guards.is_empty() {
            let body_typed = infer_expr_hinted(
                &clause_inner.body.node,
                &clause_inner.body.span,
                &mut clause_env,
                ctx,
                false,
                None,
            )
            .ok()?;

            if let Some((enum_n, var_n, payload)) = classify_variant(&body_typed.kind) {
                // Sem guards = condição True. None = não adiciona info útil.
                entries.push((None, enum_n, var_n, payload));
            } else {
                return None;
            }
            continue;
        }

        // Percorre guards da cláusula.
        let mut prev_conditions: Vec<TypedExpr> = Vec::new();
        for guard in &desugared_guards {
            if let Some(cond) = &guard.condition {
                // Guard com condition explícita.
                let cond_typed =
                    match infer_expr(&cond.node, &cond.span, &mut clause_env, ctx, false) {
                        Ok(t) => t,
                        Err(_) => {
                            return None;
                        }
                    };
                if cond_typed.ty != Ty::boolean() {
                    return None;
                }

                // Tipa o body para classificar o variant.
                let body_typed = match infer_expr_hinted(
                    &guard.body.node,
                    &guard.body.span,
                    &mut clause_env,
                    ctx,
                    false,
                    None,
                ) {
                    Ok(t) => t,
                    Err(_) => {
                        return None;
                    }
                };

                if let Some((enum_n, var_n, payload)) = classify_variant(&body_typed.kind) {
                    entries.push((Some(cond_typed.clone()), enum_n, var_n, payload));
                    prev_conditions.push(cond_typed);
                } else {
                    return None;
                }
            } else {
                // Otherwise — condição implícita = negação da disjunção dos
                // guards anteriores da mesma cláusula.
                let body_typed = match infer_expr_hinted(
                    &guard.body.node,
                    &guard.body.span,
                    &mut clause_env,
                    ctx,
                    false,
                    None,
                ) {
                    Ok(t) => t,
                    Err(_) => {
                        return None;
                    }
                };

                if let Some((enum_n, var_n, payload)) = classify_variant(&body_typed.kind) {
                    let implicit_cond = build_not_of_disjunction(&prev_conditions);
                    entries.push((implicit_cond, enum_n, var_n, payload));
                } else {
                    return None;
                }
            }
        }
    }

    // 4. Agrupa por (enum_name, variant). Para cada variant V:
    //    Post-cond de V = disjunção das condições dos guards que produzem V.
    //    payload de V = payload do variant (se todos os guards que produzem V
    //    têm o mesmo payload; None caso contrário — conservador).
    #[allow(clippy::type_complexity)] // grouping map for post-condition analysis — single use
    let mut by_variant: HashMap<
        (String, String),
        (Vec<Option<TypedExpr>>, Vec<Option<TypedExpr>>),
    > = HashMap::new();
    for (cond, enum_n, var_n, payload) in entries {
        let (conds, payloads) = by_variant.entry((enum_n, var_n)).or_default();
        conds.push(cond);
        payloads.push(payload);
    }

    let mut result = Vec::new();
    for ((enum_n, var_n), (conds, payloads)) in &by_variant {
        // Filtra None (condição True — não adiciona info como path condition).
        let conds_filtered: Vec<&TypedExpr> = conds.iter().flatten().collect();
        if conds_filtered.is_empty() {
            continue;
        }

        let condition = if conds_filtered.len() == 1 {
            conds_filtered[0].clone()
        } else {
            build_disjunction(&conds_filtered)
        };

        // Payload: só usa se todos os guards que produzem este variant têm
        // o mesmo payload (None = unitária ou payloads divergentes).
        let payload = unify_payloads(payloads);

        result.push(PostCondition {
            enum_name: enum_n.clone(),
            variant: var_n.clone(),
            condition,
            param_names: param_names.clone(),
            param_types: func_def.param_types.clone(),
            payload,
        });
    }

    Some(result)
}

/// Verifica se o tipo de retorno é um enum e retorna seu nome.
/// `Ty::Sum(name)` → `Some(name)`, `Ty::Generic(name, _)` → `Some(name)`.
fn return_type_enum_name(ret: &Ty) -> Option<String> {
    match ret {
        Ty::Sum(name) => Some(name.clone()),
        Ty::Generic(name, _) => Some(name.clone()),
        _ => None,
    }
}

/// Extrai nomes dos parâmetros dos patterns da primeira cláusula.
/// `Pattern::Ident(name)` → `name`, `Pattern::TypedIdent { name, .. }` → `name`.
/// Outros (wildcard, literal) → `__param_{i}`.
fn extract_param_names(patterns: &[Spanned<Pattern>]) -> Vec<String> {
    patterns
        .iter()
        .enumerate()
        .map(|(i, pat)| match &pat.node {
            Pattern::Ident(name) => name.clone(),
            Pattern::TypedIdent { name, .. } => name.clone(),
            _ => format!("__param_{i}"),
        })
        .collect()
}

/// Unifica payloads de múltiplos guards que produzem o mesmo variant.
/// Retorna `Some(expr)` se todos os payloads são iguais (ou se há apenas um).
/// Retorna `None` se payloads divergem ou se todos são None (unitária).
fn unify_payloads(payloads: &[Option<TypedExpr>]) -> Option<TypedExpr> {
    let first = payloads.iter().flatten().next()?;
    for p in payloads.iter().flatten() {
        // Compara por kind (ignora span/tail_pos/etc).
        if !typed_expr_eq(first, p) {
            return None;
        }
    }
    Some(first.clone())
}

/// Comparação estrutural de TypedExpr por kind (sem comparar span/metadata).
fn typed_expr_eq(a: &TypedExpr, b: &TypedExpr) -> bool {
    match (&a.kind, &b.kind) {
        (TypedExprKind::Ident { name: na }, TypedExprKind::Ident { name: nb }) => na == nb,
        (TypedExprKind::IntLit { text: ta }, TypedExprKind::IntLit { text: tb }) => ta == tb,
        (TypedExprKind::FloatLit { text: ta }, TypedExprKind::FloatLit { text: tb }) => ta == tb,
        (TypedExprKind::TextLit { text: ta }, TypedExprKind::TextLit { text: tb }) => ta == tb,
        (
            TypedExprKind::Closure {
                callee: ca,
                args: aa,
                ffi_symbol: fa,
            },
            TypedExprKind::Closure {
                callee: cb,
                args: ab,
                ffi_symbol: fb,
            },
        ) => {
            typed_expr_eq(&ca.node, &cb.node)
                && aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(x, y)| typed_expr_eq(&x.node, &y.node))
                && fa == fb
        }
        (TypedExprKind::Grouping { inner: ia }, TypedExprKind::Grouping { inner: ib }) => {
            typed_expr_eq(&ia.node, &ib.node)
        }
        (
            TypedExprKind::VariantConstruct {
                enum_name: ea,
                variant: va,
                payload: pa,
                ..
            },
            TypedExprKind::VariantConstruct {
                enum_name: eb,
                variant: vb,
                payload: pb,
                ..
            },
        ) => ea == eb && va == vb && typed_expr_eq(&pa.node, &pb.node),
        _ => false,
    }
}

/// Classifica um `TypedExprKind` em `(enum_name, variant, payload)`.
/// `payload` é `Some(expr)` para `VariantConstruct` (variante com payload),
/// `None` para `VariantQual` (variante unitária).
/// Retorna `None` se o kind não é um variant de enum.
fn classify_variant(kind: &TypedExprKind) -> Option<(String, String, Option<TypedExpr>)> {
    match kind {
        TypedExprKind::VariantConstruct {
            enum_name,
            variant,
            payload,
            ..
        } => Some((
            enum_name.clone(),
            variant.clone(),
            Some(payload.node.clone()),
        )),
        TypedExprKind::VariantQual {
            enum_name, variant, ..
        } => Some((enum_name.clone(), variant.clone(), None)),
        _ => None,
    }
}

/// Constrói `not(or(f1, f2, ...))` a partir de uma lista de condições.
/// Se a lista é vazia, retorna `None` (sem condição — otherwise sem guards
/// anteriores é sempre True).
fn build_not_of_disjunction(conds: &[TypedExpr]) -> Option<TypedExpr> {
    if conds.is_empty() {
        return None;
    }
    let disjunction = if conds.len() == 1 {
        conds[0].clone()
    } else {
        let refs: Vec<&TypedExpr> = conds.iter().collect();
        build_disjunction(&refs)
    };
    Some(build_not(disjunction))
}

/// Constrói `or(f1, f2, ...)` a partir de uma lista de condições.
/// Left-fold: `or(or(f1, f2), f3), ...`
fn build_disjunction(conds: &[&TypedExpr]) -> TypedExpr {
    debug_assert!(!conds.is_empty());
    if conds.len() == 1 {
        return conds[0].clone();
    }
    let mut acc = build_or(conds[0], conds[1]);
    for cond in &conds[2..] {
        acc = build_or(&acc, cond);
    }
    acc
}

/// Constrói `not(expr)` como `TypedExpr`.
fn build_not(expr: TypedExpr) -> TypedExpr {
    let span = expr.span;
    TypedExpr {
        span,
        ty: Ty::boolean(),
        tail_pos: false,
        escape: expr.escape,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(
                TypedExpr {
                    span,
                    ty: Ty::boolean(),
                    tail_pos: false,
                    escape: expr.escape,
                    kind: TypedExprKind::Ident {
                        name: "not".to_string(),
                    },
                },
                span,
            )),
            args: vec![Spanned::new(expr, span)],
            ffi_symbol: None,
        },
    }
}

/// Constrói `or(a, b)` como `TypedExpr`.
fn build_or(a: &TypedExpr, b: &TypedExpr) -> TypedExpr {
    let span = a.span;
    TypedExpr {
        span,
        ty: Ty::boolean(),
        tail_pos: false,
        escape: a.escape,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(
                TypedExpr {
                    span,
                    ty: Ty::boolean(),
                    tail_pos: false,
                    escape: a.escape,
                    kind: TypedExprKind::Ident {
                        name: "or".to_string(),
                    },
                },
                span,
            )),
            args: vec![Spanned::new(a.clone(), span), Spanned::new(b.clone(), span)],
            ffi_symbol: None,
        },
    }
}

/// Substitui parâmetros por argumentos numa condition.
///
/// Para cada `Ident(name)` na expr, se `name` corresponde a `param_names[i]`,
/// substitui por `args[i].node.clone()`. Recursiva em Closure e Grouping.
/// Outros nós (IntLit, FloatLit, etc.) não contêm Idents — mantém como está.
pub(crate) fn substitute_params(
    expr: &TypedExpr,
    param_names: &[String],
    args: &[Spanned<TypedExpr>],
) -> TypedExpr {
    match &expr.kind {
        TypedExprKind::Ident { name } => {
            if let Some(idx) = param_names.iter().position(|p| p == name) {
                if idx < args.len() {
                    args[idx].node.clone()
                } else {
                    expr.clone()
                }
            } else {
                expr.clone()
            }
        }
        TypedExprKind::Closure {
            callee,
            args: closure_args,
            ffi_symbol,
        } => TypedExpr {
            span: expr.span,
            ty: expr.ty.clone(),
            tail_pos: expr.tail_pos,
            escape: expr.escape,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(
                    substitute_params(&callee.node, param_names, args),
                    callee.span,
                )),
                args: closure_args
                    .iter()
                    .map(|a| Spanned::new(substitute_params(&a.node, param_names, args), a.span))
                    .collect(),
                ffi_symbol: ffi_symbol.clone(),
            },
        },
        TypedExprKind::Grouping { inner } => TypedExpr {
            span: expr.span,
            ty: expr.ty.clone(),
            tail_pos: expr.tail_pos,
            escape: expr.escape,
            kind: TypedExprKind::Grouping {
                inner: Box::new(Spanned::new(
                    substitute_params(&inner.node, param_names, args),
                    inner.span,
                )),
            },
        },
        _ => expr.clone(),
    }
}

// ── Inlining de funções puras no Z3 translator (§9.10) ──────────────

/// Corpo tipado de uma função pura, para inlining no Z3 translator.
///
/// Quando o translator encontra `Closure { Ident(f), args }` e `f` está
/// nesta tabela, ele substitui os params pelos args (`substitute_params`)
/// e traduz o resultado. Isso torna funções puras com corpo Kata
/// transparentes para o Z3 — sem codificar semântica de cada função
/// individualmente no translator.
#[derive(Debug, Clone)]
pub(crate) struct InlineFnBody {
    /// Nomes dos parâmetros (da primeira cláusula).
    pub param_names: Vec<String>,
    /// Tipos dos parâmetros (para desambiguar overloads).
    pub param_types: Vec<Ty>,
    /// Corpo tipado da cláusula (quando há uma única cláusula sem guards).
    /// `None` se a função tem múltiplas cláusulas ou guards — não inlinable.
    pub body: Option<TypedExpr>,
}

/// Tabela de funções puras inlinable, por nome.
#[derive(Debug, Clone, Default)]
pub(crate) struct InlineFnTable {
    map: HashMap<String, Vec<InlineFnBody>>,
}

impl InlineFnTable {
    /// Consulta o corpo tipado de uma função pelo nome, escolhendo o
    /// overload cujos param types são compatíveis com os args da chamada.
    pub(crate) fn get(&self, name: &str, arg_types: &[Ty]) -> Option<&InlineFnBody> {
        let overloads = self.map.get(name)?;
        // Match por arity + compatibilidade de tipos.
        overloads.iter().find(|body| {
            body.param_types.len() == arg_types.len()
                && body
                    .param_types
                    .iter()
                    .zip(arg_types.iter())
                    .all(|(param, arg)| param == arg)
        })
    }

    /// True se a tabela está vazia.
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Extrai corpos tipados de funções puras simples (uma cláusula, sem guards,
/// sem `@ffi`) para inlining no Z3 translator.
///
/// Segue o mesmo padrão de `extract_post_conditions`: usa um `InferCtx`
/// temporário para tipar o corpo. Conservador: se a tipagem falha ou a
/// função não é simples, skip.
pub(crate) fn extract_inline_bodies(
    functions: &[FunctionDef],
    ctx: &InferCtx,
    module_type_env: &TypeEnv,
) -> InlineFnTable {
    let mut map: HashMap<String, Vec<InlineFnBody>> = HashMap::new();

    for func_def in functions {
        // Só inlina funções puras simples: uma cláusula, sem guards.
        if func_def.clauses.len() != 1 {
            continue;
        }
        let clause = &func_def.clauses[0].node;
        if !clause.guards.is_empty() {
            continue;
        }

        let param_names = extract_param_names(&clause.patterns);

        // Tipa o body usando um escopo filho com os tipos do módulo.
        let mut clause_env = TypeEnv::with_parent(module_type_env.clone());

        // Desugar o body (elimina Pipe/Hole antes do typeck).
        let desugared_body = crate::desugar::desugar(&clause.body);

        // Check patterns para definir bindings no escopo da cláusula.
        if check_patterns(
            &clause.patterns,
            &func_def.param_types,
            ctx.enum_registry,
            &mut clause_env,
            ctx.interface_registry,
            ctx.struct_registry,
            ctx.refined_decls,
        )
        .is_err()
        {
            continue;
        }

        // Process with bindings (pode afetar tipagem dos guards).
        let desugared_with_bindings: Vec<kata_ast::WithBinding> = clause
            .with_bindings
            .iter()
            .map(|w| kata_ast::WithBinding {
                name: w.name.clone(),
                value: crate::desugar::desugar(&w.value),
            })
            .collect();
        if process_with_bindings(&desugared_with_bindings, &mut clause_env, ctx).is_err() {
            continue;
        }

        // Tipa o body.
        let body_typed = infer_expr_hinted(
            &desugared_body.node,
            &desugared_body.span,
            &mut clause_env,
            ctx,
            false,
            None,
        );

        match body_typed {
            Ok(typed) => {
                map.entry(func_def.name.clone())
                    .or_default()
                    .push(InlineFnBody {
                        param_names,
                        param_types: func_def.param_types.clone(),
                        body: Some(typed),
                    });
            }
            Err(_) => continue,
        }
    }

    InlineFnTable { map }
}
