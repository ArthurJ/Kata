//! Núcleo da inferência de expressões — o grande match sobre `Expr`.
//!
//! `infer_expr` é o entry point público (usado por todos os submódulos).
//! `infer_expr_hinted` aceita um type hint opcional (DoD 29) para inferência
//! bidirecional top-down.

use std::cell::RefCell;
use std::collections::HashMap;

use kata_ast::{Expr, GuardClause, Pattern, Span, Spanned, TypeExpr, WithBinding};
use kata_core::StructKey;
use kata_core::dispatch::{DispatchTable, match_score};
use kata_core::enum_registry::EnumRegistry;
use kata_core::escape::EscapeTarget;
use kata_core::interface_registry::InterfaceRegistry;
use kata_core::struct_registry::StructRegistry;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use kata_resolution::RefinedDeclInfo;

use crate::typed::{TypedExpr, TypedExprKind};

use super::_match::infer_match;
use super::action_call::infer_action_call;
use super::apply::infer_apply;
use super::dot_access::infer_dot_access;
use super::helpers::InferResult;
use super::lambda::infer_lambda;
use super::sugar::{infer_pipe_fallback, infer_pipe_limit, infer_question};
use super::variant::resolve_unqual_variant;

/// Lambda deferido para use-site inference.
///
/// Quando `infer_lambda` não consegue resolver os tipos dos params
/// (partial dispatch ambíguo, sem hint), o lambda é guardado aqui
/// com `InferVar` nos param types. Quando a função é aplicada
/// (`f 5 3`), `infer_apply` consulta esta table e re-inere o lambda
/// com os arg types reais via `infer_apply_lambda`.
#[derive(Debug, Clone)]
pub(crate) struct DeferredLambda {
    pub patterns: Vec<Spanned<Pattern>>,
    pub body: Box<Spanned<Expr>>,
    pub guards: Vec<GuardClause>,
    pub with_bindings: Vec<WithBinding>,
}

/// Type alias para a side table de lambdas deferidos.
pub(crate) type DeferredLambdaTable = RefCell<HashMap<String, DeferredLambda>>;

/// Contexto de inferência — carrega dependências compartilhadas entre
/// todas as funções de inferência. Substitui parâmetros individuais
/// `table` e `enum_registry`, e adiciona `ret_ty` para validação de
/// `return` em Actions.
pub(crate) struct InferCtx<'a> {
    pub table: &'a DispatchTable,
    pub enum_registry: &'a EnumRegistry,
    /// Catálogo de structs com campos — para field access e
    /// ascription-construção.
    pub struct_registry: &'a StructRegistry,
    /// Declarações refined para ascription-refined (validação
    /// compile-time de predicados sobre literais).
    pub refined_decls: &'a [RefinedDeclInfo],
    /// Catálogo de interfaces e implementações para dispatch
    /// com `iface++` no Score.
    pub interface_registry: &'a InterfaceRegistry,
    /// Catálogo de delegações `refines` — fallback no dispatch:
    /// substitui args refined pelo tipo base e retenta.
    pub refines_registry: &'a kata_core::RefinesRegistry,
    /// Tipo de retorno da Action atual — `Some(ty)` quando inferindo
    /// o body de uma Action, `None` caso contrário. Usado por `infer_return`
    /// para verificar que `return expr` produz o tipo esperado.
    pub ret_ty: Option<&'a Ty>,
    /// `true` quando inferindo dentro de um `loop`. Usado por `infer_break`
    /// e `infer_continue` para validar que só aparecem dentro de loop.
    pub in_loop: bool,
    /// Side table de lambdas deferidos para use-site inference.
    /// Quando `let f := lambda a b: - a b` falha (partial dispatch ambíguo),
    /// o lambda é guardado aqui. Quando `f 5 3` é aplicado, `infer_apply`
    /// consulta esta table e re-inere o lambda com os arg types reais.
    pub deferred_lambdas: &'a DeferredLambdaTable,
    /// Path conditions — facts booleanos acumulados de guards e
    /// pattern matches no escopo atual. Usados para provar
    /// ascriptions refinadas sobre não-literais via Z3.
    /// Nível 1: guards locais apenas (PRD-refinement-propagation).
    pub path_conditions: super::path_conditions::PathConditionCtx,
    /// Post-condições inter-procedurais — Nível 2.
    /// Mapa func_name → post-condições extraídas dos guards do corpo
    /// da função. Consumido no visitor de match quando o scrutinee é
    /// uma chamada de função (Closure).
    pub post_conds: &'a super::post_conditions::PostCondTable,
    /// Corpos tipados de funções puras inlinable, para o Z3 translator.
    /// Quando o translator encontra `Closure { Ident(f), args }` e `f`
    /// está nesta tabela, inlina o corpo (substituindo params por args)
    /// e traduz o resultado. Torna funções puras com corpo Kata
    /// transparentes para o Z3 (ex: `zero`).
    pub inline_fns: &'a super::post_conditions::InlineFnTable,
}

/// Infere o tipo de uma expressão, produzindo um `TypedExpr`.
///
/// Verifica se um TypedExprKind contém `break` (recursivamente).
/// Usado para determinar se um `loop` pode completar normalmente (tem break)
/// ou é divergente (só sai via return).
fn contains_break(kind: &TypedExprKind) -> bool {
    match kind {
        TypedExprKind::Break => true,
        TypedExprKind::Loop { .. } => {
            // break em loop interno não conta — pertence ao loop interno.
            false
        }
        TypedExprKind::Match { scrutinee, arms } => {
            arms.iter().any(|arm| contains_break(&arm.body.node.kind))
                || arms.iter().any(|arm| {
                    arm.guard
                        .as_ref()
                        .is_some_and(|g| contains_break(&g.node.kind))
                })
                || contains_break(&scrutinee.node.kind)
        }
        TypedExprKind::Let { value, .. } => contains_break(&value.node.kind),
        TypedExprKind::Var { value, .. } => contains_break(&value.node.kind),
        TypedExprKind::Return(inner) => contains_break(&inner.node.kind),
        _ => false,
    }
}

/// `tail_pos` é `true` quando a expressão está em posição de cauda. O entry
/// point é sempre `tail_pos = true`. Sub-expressões de `Let` value são
/// `tail_pos = false`. Argumentos de `Apply` são `tail_pos = false`.
/// Body de lambda em tail position é `tail_pos = true`. Body de match arm
/// em tail position é `tail_pos = true`.
pub(crate) fn infer_expr(
    expr: &Expr,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    infer_expr_hinted(expr, span, env, ctx, tail_pos, None)
}

/// Verifica se `actual` cabe em `declared` — direcional (não simétrica).
/// `Var("T")` no actual significa "não-constrangido" e aceita o declarado.
/// Recursiva dentro de `Generic`.
pub(crate) fn fits_return(actual: &Ty, declared: &Ty) -> bool {
    match (actual, declared) {
        (Ty::Var(_), _) => true,
        // `return Err(e)` from `?` desugar: Result with unresolved type param
        // is a bottom/divergent expression — accept regardless of declared return type.
        (Ty::Generic(n, args), _)
            if n == "Result" && args.len() == 2 && matches!(args[0], Ty::Var(_)) =>
        {
            true
        }
        // Instance de família polimórfica é compatível com Family da mesma família.
        // Ex: construtor NonZeroPoly(3) retorna Result::(Instance("NonZeroPoly", "Int"), Text),
        // mas a action declara Result::(NonZeroPoly, Text) onde NonZeroPoly resolve para Family.
        (
            Ty::Struct(StructKey::Instance(family, _)),
            Ty::Struct(StructKey::Family(decl_family)),
        ) if family == decl_family => true,
        (Ty::Generic(n1, a1), Ty::Generic(n2, a2)) if n1 == n2 && a1.len() == a2.len() => {
            a1.iter().zip(a2).all(|(x, y)| fits_return(x, y))
        }
        // Tipos unários (List, Array, Range, Sender, Receiver, ReceiverFactory)
        // — recursão para o tipo interno, permitindo Var casar com concreto.
        (Ty::List(a), Ty::List(b))
        | (Ty::Array(a), Ty::Array(b))
        | (Ty::Range(a), Ty::Range(b))
        | (Ty::Sender(a), Ty::Sender(b))
        | (Ty::Receiver(a), Ty::Receiver(b))
        | (Ty::ReceiverFactory(a), Ty::ReceiverFactory(b))
        | (Ty::Set(a), Ty::Set(b)) => fits_return(a, b),
        // Tipos binários (Dict) — recursão para K e V.
        (Ty::Dict(k1, v1), Ty::Dict(k2, v2)) => fits_return(k1, k2) && fits_return(v1, v2),
        _ => actual == declared,
    }
}

///
/// When `hint` is `Some(Ty::Function(params, ret))` and `expr` is a `Lambda`,
/// the params are used as the lambda's parameter types instead of InferVar.
/// When `hint` is `Some(ty)` and `expr` is a `TypeAscription`, the hint is
/// propagated to the inner expression (ascription already provides a target
/// type, so the hint is redundant there but harmless).
#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_expr_hinted(
    expr: &Expr,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
    hint: Option<&Ty>,
) -> InferResult<TypedExpr> {
    let (ty, kind) = match expr {
        // ── Literais ─────────────────────────────────────────
        Expr::IntLit { text } => (Ty::int(), TypedExprKind::IntLit { text: text.clone() }),
        Expr::FloatLit { text } => (Ty::float(), TypedExprKind::FloatLit { text: text.clone() }),
        Expr::TextLit { text } => (Ty::text(), TypedExprKind::TextLit { text: text.clone() }),
        Expr::BytesLit { bytes } => (
            Ty::Bytes,
            TypedExprKind::BytesLit {
                bytes: bytes.clone(),
            },
        ),
        Expr::Unit => (Ty::Unit, TypedExprKind::Unit),

        // ── Identificador ────────────────────────────────────
        Expr::Ident { name } => {
            // Caminho 1: variável local no TypeEnv.
            if let Some(ty) = env.lookup(name).cloned() {
                (ty, TypedExprKind::Ident { name: name.clone() })
            } else {
                // Caminho 2: variante unitária desqualificada (ex: `True`,
                // `None`, `Vermelho`). Busca no EnumRegistry.
                match resolve_unqual_variant(name, span, ctx) {
                    Ok(result) => result,
                    Err(MiddleError::UnboundName {
                        name: ref err_name, ..
                    }) if !err_name.contains("ambí") && !err_name.contains("payload") => {
                        // Caminho 3: função no DispatchTable (first-class reference).
                        // `worker` sem `!` referencia a Action como valor.
                        // `+`, `*`, `<` em Grouping `(+)` referenciam função despachada.
                        if let Some(overloads) = ctx.table.get_overloads(name) {
                            let action_overloads: Vec<_> =
                                overloads.iter().filter(|o| o.is_action).collect();
                            if !action_overloads.is_empty() {
                                // Desambiguação por hint (segunda versão do PRD §12).
                                // Se há múltiplos overloads, usa o hint de tipo esperado
                                // para selecionar o compatível. Sem hint, produz
                                // Ty::OverloadSet para resolver no call site (dispatch por args).
                                if let Some(overload) =
                                    select_action_overload(&action_overloads, hint, ctx)
                                {
                                    // hint resolveu — Ty::Action concreto
                                    (
                                        Ty::Action(
                                            overload.params.clone(),
                                            Box::new(overload.ret.clone()),
                                        ),
                                        TypedExprKind::Ident { name: name.clone() },
                                    )
                                } else {
                                    // Sem hint ou múltiplos compatíveis — Ty::OverloadSet
                                    let overloads: Vec<(Vec<Ty>, Ty)> = action_overloads
                                        .iter()
                                        .map(|o| (o.params.clone(), o.ret.clone()))
                                        .collect();
                                    (
                                        Ty::OverloadSet {
                                            name: name.clone(),
                                            overloads,
                                        },
                                        TypedExprKind::Ident { name: name.clone() },
                                    )
                                }
                            } else {
                                // Caminho 3b: função despachada não-action (ex: `+`, `*`, `<`).
                                // Quando aparece standalone (ex: callback de HOF em Grouping),
                                // trata como valor de função. Se há um único overload, retorna
                                // Ty::Function. Se múltiplos, Ty::OverloadSet para dispatch no
                                // call site.
                                let fn_overloads: Vec<_> =
                                    overloads.iter().filter(|o| !o.is_action).collect();
                                if fn_overloads.len() == 1 {
                                    let o = fn_overloads[0];
                                    (
                                        Ty::Function(o.params.clone(), Box::new(o.ret.clone())),
                                        TypedExprKind::Ident { name: name.clone() },
                                    )
                                } else if !fn_overloads.is_empty() {
                                    let overloads: Vec<(Vec<Ty>, Ty)> = fn_overloads
                                        .iter()
                                        .map(|o| (o.params.clone(), o.ret.clone()))
                                        .collect();
                                    (
                                        Ty::OverloadSet {
                                            name: name.clone(),
                                            overloads,
                                        },
                                        TypedExprKind::Ident { name: name.clone() },
                                    )
                                } else {
                                    // Caminho 4: realmente unbound.
                                    return Err(MiddleError::UnboundName {
                                        name: name.clone(),
                                        span: (*span).into(),
                                        suggestion: None,
                                    });
                                }
                            }
                        } else {
                            return Err(MiddleError::UnboundName {
                                name: name.clone(),
                                span: (*span).into(),
                                suggestion: None,
                            });
                        }
                    }
                    Err(other) => return Err(other),
                }
            }
        }

        // ── Aplicação prefixa ────────────────────────────────
        // Propaga hint para infer_apply (ret-directed dispatch).
        Expr::Apply { callee, args } => infer_apply(callee, args, span, env, ctx, hint)?,

        // ── Ascription de tipo ───────────────────────────────
        Expr::TypeAscription { expr, ty } => {
            return super::ascription::infer_type_ascription(
                expr, ty, span, env, ctx, tail_pos, hint,
            );
        }

        // ── Grouping — transparente, propaga hint ────────────
        Expr::Grouping { inner } => {
            let typed_inner =
                infer_expr_hinted(&inner.node, &inner.span, env, ctx, tail_pos, hint)?;
            (
                typed_inner.ty.clone(),
                TypedExprKind::Grouping {
                    inner: Box::new(Spanned::new(typed_inner, inner.span)),
                },
            )
        }

        // ── Tuple ────────────────────────────────────────────
        Expr::Tuple { elements } => {
            let mut typed_elements = Vec::with_capacity(elements.len());
            let mut element_tys = Vec::with_capacity(elements.len());
            for elem in elements {
                let typed = infer_expr(&elem.node, &elem.span, env, ctx, false)?;
                element_tys.push(typed.ty.clone());
                typed_elements.push(Spanned::new(typed, elem.span));
            }
            (
                Ty::Tuple(element_tys),
                TypedExprKind::Tuple {
                    elements: typed_elements,
                },
            )
        }

        // ── Let binding ──────────────────────────────────────
        Expr::Let { name, value } => {
            // `let` é imutável e único por escopo — re-declaração no mesmo
            // escopo é erro. Exceções:
            // - `_` (wildcard) — significa "descartar resultado".
            // - Nomes prefixados com `_` — sintéticos injetados pelo desugar
            //   de diretivas (`_name`, `_arity`, `_types`, `_return_type`,
            //   `_is_action`, `_args`, `_return`). Múltiplas diretivas
            //   empilhadas reutilizam os mesmos nomes.
            // Shadowing de params/constantes (escopos pai) é permitido.
            // Para reusar um nome, use `var` (re-binding explícito).
            if !name.starts_with('_') && env.is_locally_defined(name) {
                return Err(MiddleError::DuplicateDecl {
                    name: name.clone(),
                    span: (*span).into(),
                });
            }
            // Tente inferir o valor. Se falha com LambdaInferenceFail e o
            // value é um lambda, deferre para use-site inference: guarda o
            // AST do lambda na side table e define o binding com InferVars.
            // Quando `f 5 3` for aplicado, infer_apply resgata o lambda e
            // re-inere com os arg types reais.
            let typed_value =
                match infer_expr_hinted(&value.node, &value.span, env, ctx, false, hint) {
                    Ok(tv) => tv,
                    Err(MiddleError::LambdaInferenceFail { .. })
                        if matches!(value.node, Expr::Lambda { .. }) =>
                    {
                        // Deferre o lambda para use-site inference.
                        if let Expr::Lambda {
                            patterns,
                            body,
                            guards,
                            with_bindings,
                        } = &value.node
                        {
                            let n_params = patterns.len();
                            let param_types: Vec<Ty> =
                                (0..n_params).map(|i| Ty::InferVar(i as u32)).collect();
                            let ret_ty = Ty::InferVar(n_params as u32);
                            let lambda_ty =
                                Ty::Function(param_types.clone(), Box::new(ret_ty.clone()));

                            // Guarda o AST do lambda na side table.
                            ctx.deferred_lambdas.borrow_mut().insert(
                                name.clone(),
                                DeferredLambda {
                                    patterns: patterns.clone(),
                                    body: body.clone(),
                                    guards: guards.clone(),
                                    with_bindings: with_bindings.clone(),
                                },
                            );

                            // Constrói um TypedExpr skeleton para o lambda deferido.
                            // O codegen verá InferVar nos param_types e tratará
                            // como placeholder — o tipo real será resolvido no uso.
                            let typed_patterns = patterns
                                .iter()
                                .enumerate()
                                .map(|(i, _)| {
                                    Spanned::new(
                                        crate::typed::TypedPattern::Ident {
                                            name: if let kata_ast::Pattern::Ident(n) =
                                                &patterns[i].node
                                            {
                                                n.clone()
                                            } else {
                                                format!("__param_{i}")
                                            },
                                            ty: Ty::InferVar(i as u32),
                                        },
                                        patterns[i].span,
                                    )
                                })
                                .collect();
                            let skeleton_kind = TypedExprKind::Lambda {
                                func_name: None,
                                param_types: param_types.clone(),
                                ret_ty: ret_ty.clone(),
                                clauses: vec![crate::typed::TypedLambdaClause {
                                    patterns: typed_patterns,
                                    body: Spanned::new(
                                        TypedExpr {
                                            span: body.span,
                                            ty: ret_ty.clone(),
                                            tail_pos: true,
                                            escape: EscapeTarget::Local,
                                            kind: TypedExprKind::Unit,
                                        },
                                        body.span,
                                    ),
                                    guards: Vec::new(),
                                    with_bindings: Vec::new(),
                                }],
                                captures: Vec::new(),
                            };
                            let skeleton = TypedExpr {
                                span: value.span,
                                ty: lambda_ty.clone(),
                                tail_pos: false,
                                escape: EscapeTarget::Local,
                                kind: skeleton_kind,
                            };

                            // Define no TypeEnv com InferVars.
                            env.define(name, lambda_ty, "__local__");

                            return Ok(TypedExpr {
                                span: *span,
                                ty: Ty::Unit,
                                tail_pos,
                                escape: EscapeTarget::Local,
                                kind: TypedExprKind::Let {
                                    name: name.clone(),
                                    value: Box::new(Spanned::new(skeleton, value.span)),
                                },
                            });
                        }
                        // Inalcançável — o guard do match garante que é Lambda.
                        return Err(MiddleError::LambdaInferenceFail {
                            span: (*span).into(),
                            detail: None,
                        });
                    }
                    Err(e) => return Err(e),
                };

            // (PRD OverloadSet): se o lambda retornou OverloadSet
            // (partial dispatch ambíguo), registra o AST do lambda na side
            // table de deferred. O infer_apply caminho 2c consulta a side
            // table quando vê OverloadSet no TypeEnv e re-infere com tipos
            // concretos dos args.
            if let Ty::OverloadSet { .. } = &typed_value.ty
                && let Expr::Lambda {
                    patterns,
                    body,
                    guards,
                    with_bindings,
                } = &value.node
            {
                ctx.deferred_lambdas.borrow_mut().insert(
                    name.clone(),
                    DeferredLambda {
                        patterns: patterns.clone(),
                        body: body.clone(),
                        guards: guards.clone(),
                        with_bindings: with_bindings.clone(),
                    },
                );
            }

            let val_ty = typed_value.ty.clone();

            // Rastrear provenance: se `let g := soma` onde `soma` é Ident
            // apontando para função nomeada no DispatchTable, marcar o
            // binding com `fn_alias = Some("soma")`. Isto permite que a
            // reflexão distinga alias (caso dinâmico, escalar via sidecar
            // table) de lambda com binding (caso estático, lista).
            //
            // Também rastreia Actions: `let f := worker` onde `worker` é
            // Action no DispatchTable. O fn_alias guarda "worker" para que o
            // caminho indirect de ActionCall produza callee = "worker"
            // (nome da action) em vez de "f" (nome da variável). Sem isto,
            // o monomorphizador não encontra a action no DispatchTable e
            // não instancia a versão genérica.
            let fn_alias = match (&value.node, &typed_value.kind) {
                (Expr::Ident { name: src_name }, _)
                    if matches!(val_ty, Ty::Function(_, _))
                        && ctx
                            .table
                            .get_overloads(src_name)
                            .is_some_and(|ols| ols.iter().any(|oi| !oi.is_action)) =>
                {
                    Some(src_name.clone())
                }
                (Expr::Ident { name: src_name }, _)
                    if matches!(val_ty, Ty::Action(_, _) | Ty::OverloadSet { .. })
                        && ctx
                            .table
                            .get_overloads(src_name)
                            .is_some_and(|ols| ols.iter().any(|oi| oi.is_action)) =>
                {
                    Some(src_name.clone())
                }
                _ => None,
            };

            if fn_alias.is_some() {
                env.define_with_alias(name, val_ty, "__local__", fn_alias);
            } else {
                env.define(name, val_ty, "__local__");
            }

            (
                Ty::Unit,
                TypedExprKind::Let {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
            )
        }
        // ── LetDestruct: let (x, y, ...) := expr ──────────────
        // Desugaring: `let __t := expr` + `let x := __t.0` + ...
        // Gera um único nó TAST que faz o binding temporário e os
        // bindings individuais via FieldAccess. O codegen processa
        // os Lets em sequência (cada um define uma variável local).
        Expr::LetDestruct { names, value } => {
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
            let val_ty = typed_value.ty.clone();
            let temp_name = "__let_destruct";

            // Verificar duplicação antes de definir qualquer binding.
            // O temporário usa nome sintético — conflito só se o usuário
            // usou `__let_destruct` explicitamente (improvável, mas seguro).
            if env.is_locally_defined(temp_name) {
                return Err(MiddleError::DuplicateDecl {
                    name: temp_name.to_string(),
                    span: (*span).into(),
                });
            }
            // Define o temporário no escopo.
            env.define(temp_name, val_ty.clone(), "__local__");

            // Para cada nome (pulando `_`), define no escopo via FieldAccess.
            let mut field_bindings: Vec<(String, Spanned<TypedExpr>)> = Vec::new();
            for (i, name) in names.iter().enumerate() {
                if name == "_" {
                    continue;
                }
                // `let` é imutável e único por escopo — mesmo em destructuring.
                if env.is_locally_defined(name) {
                    return Err(MiddleError::DuplicateDecl {
                        name: name.clone(),
                        span: (*span).into(),
                    });
                }
                // Tipo do elemento i da tupla.
                let elem_ty = match &val_ty {
                    Ty::Tuple(tys) => tys.get(i).cloned().unwrap_or(Ty::Unit),
                    _ => {
                        return Err(MiddleError::TypeMismatch {
                            expected: "tupla para destructuring".into(),
                            found: format!("{}", val_ty),
                            span: value.span.into(),
                        });
                    }
                };
                env.define(name, elem_ty.clone(), "__local__");

                // Constrói FieldAccess: __let_destruct.i
                let temp_expr = TypedExpr {
                    span: Span::synthetic(),
                    ty: val_ty.clone(),
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    kind: TypedExprKind::Ident {
                        name: temp_name.to_string(),
                    },
                };
                let access = TypedExpr {
                    span: Span::synthetic(),
                    ty: elem_ty,
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    kind: TypedExprKind::FieldAccess {
                        expr: Box::new(Spanned::new(temp_expr, Span::synthetic())),
                        struct_name: String::new(),
                        field_name: String::new(),
                        field_index: i as u32,
                    },
                };
                field_bindings.push((name.clone(), Spanned::new(access, Span::synthetic())));
            }

            (
                Ty::Unit,
                TypedExprKind::LetDestruct {
                    temp_name: temp_name.to_string(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                    bindings: field_bindings,
                },
            )
        }

        // ── Qualificação de variante (sem Apply = unitária) ─────
        // `Ident :: Ident` é ambíguo no parser: pode ser VariantQual
        // (Result::Ok) ou TypeAscription (a::Int — downcast refined→base).
        // Tentar VariantQual primeiro; se falhar (não é enum), retentar
        // como TypeAscription onde enum_name é a variável e variant é o
        // tipo alvo.
        Expr::VariantQual {
            enum_name,
            variant,
            module_path,
            ..
        } => {
            // Quando module_path qualifica (ex: core.Result::Err), resolve o
            // enum_ty do módulo de origem, não o do escopo mais próximo.
            let enum_ty = if let Some(path) = module_path.as_ref()
                && let Some(first) = path.first()
            {
                env.lookup_with_origin(enum_name, first).cloned()
            } else {
                env.lookup(enum_name).cloned()
            };

            // Caminho 1: é uma variante de enum.
            if let Some(ref ty) = enum_ty
                && let Some((vt, vk)) = super::variant_qual::infer_variant_qual(
                    enum_name,
                    variant,
                    module_path.as_deref(),
                    ty,
                    span,
                    ctx,
                )?
            {
                let escape = if ctx.ret_ty.is_some() {
                    if tail_pos {
                        EscapeTarget::Caller
                    } else {
                        EscapeTarget::Local
                    }
                } else {
                    EscapeTarget::Caller
                };
                return Ok(TypedExpr {
                    span: *span,
                    ty: vt,
                    tail_pos,
                    escape,
                    kind: vk,
                });
            }

            // Caminho 2: TypeAscription disfarçada — `var::Type`.
            // O parser produziu VariantQual porque ambos os lados são Ident.
            // Se enum_name existe no env como variável e variant é um tipo
            // conhecido, tratar como TypeAscription.
            if enum_ty.is_some() {
                let type_expr = Spanned::new(TypeExpr::Named(variant.clone()), *span);
                let inner_expr = Spanned::new(
                    Expr::Ident {
                        name: enum_name.clone(),
                    },
                    *span,
                );
                let result = super::ascription::infer_type_ascription(
                    &inner_expr,
                    &type_expr,
                    span,
                    env,
                    ctx,
                    tail_pos,
                    hint,
                )?;
                return Ok(result);
            }

            // Nem variante nem variável — unbound.
            Err(MiddleError::UnboundName {
                name: enum_name.clone(),
                span: (*span).into(),
                suggestion: suggest_similar(enum_name, ctx.table.all_names()),
            })?
        }

        // ── Desugared antes do typeck ──────────────────
        Expr::Hole => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (Hole deve ter sido desugared)".into(),
                found: "Hole".into(),
                span: (*span).into(),
            });
        }
        Expr::Pipe { .. } => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (Pipe deve ter sido desugared)".into(),
                found: "Pipe".into(),
                span: (*span).into(),
            });
        }
        Expr::PipeLimit { lhs, rhs, limit } => {
            let result = infer_pipe_limit(lhs, rhs, limit, span, env, ctx)?;
            (result.ty, result.kind)
        }

        // ── Lambda ──────────────────────────────
        Expr::Lambda {
            patterns,
            body,
            guards,
            with_bindings,
        } => infer_lambda(patterns, body, guards, with_bindings, span, env, ctx, hint)?,

        // ── Match ───────────────────────────────
        Expr::Match { scrutinee, arms } => {
            infer_match(scrutinee, arms, span, env, ctx, tail_pos, hint)?
        }

        // ── ActionCall — dispatch para Action builtin ou definida ──
        Expr::ActionCall { callee, args } => {
            match infer_action_call(callee, args, span, env, ctx)? {
                super::action_call::ActionDispatch::Complete(typed) => return Ok(typed),
                super::action_call::ActionDispatch::Tuple(ty, kind) => (ty, kind),
            }
        }

        // ── Var — binding mutável (exclusivo de Actions) ──
        Expr::Var { name, value } => {
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
            let val_ty = typed_value.ty.clone();
            env.define_mutable(name, val_ty, "__local__");
            (
                Ty::Unit,
                TypedExprKind::Var {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
            )
        }

        // ── Reassign — reatribuição a variável `var` ──
        Expr::Reassign { name, value } => {
            // Verifica que a variável existe e foi declarada como mutável.
            let existing_ty =
                env.lookup(name)
                    .cloned()
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: name.clone(),
                        span: (*span).into(),
                        suggestion: None,
                    })?;
            if !env.is_mutable(name) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("variável mutável `{name}` (declarada com `var`)"),
                    found: format!("variável imutável `{name}` (declarada com `let`)"),
                    span: (*span).into(),
                });
            }
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
            if typed_value.ty != existing_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{existing_ty:?}"),
                    found: format!("{}", typed_value.ty),
                    span: value.span.into(),
                });
            }
            (
                Ty::Unit,
                TypedExprKind::Reassign {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
            )
        }

        // ── Return — early return de Action ──
        Expr::Return(inner) => {
            let ret_ty = ctx.ret_ty.ok_or_else(|| MiddleError::TypeMismatch {
                expected: "return dentro de Action".into(),
                found: "return fora de Action".into(),
                span: (*span).into(),
            })?;
            let expanded_ret = ctx.enum_registry.expand_defaults(ret_ty);
            let typed_inner = infer_expr(&inner.node, &inner.span, env, ctx, false)?;
            if !fits_return(&typed_inner.ty, &expanded_ret) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{ret_ty:?}"),
                    found: format!("{}", typed_inner.ty),
                    span: inner.span.into(),
                });
            }
            (
                typed_inner.ty.clone(),
                TypedExprKind::Return(Box::new(Spanned::new(typed_inner, inner.span))),
            )
        }
        Expr::Loop { body } => {
            // Loop body é inferido com in_loop = true.
            // Cada expr do body é inferida em sequência no mesmo escopo.
            //
            // Se o loop contém `break`, ele pode completar normalmente — o tipo
            // é Unit (break sem valor). Se NÃO contém `break`, o loop só sai
            // via `return` (que escapa da action) — é divergente. Nesse caso
            // o tipo é irrelevante: usamos Ty::Var para que fits_return aceite
            // contra qualquer tipo de retorno declarado.
            let loop_ctx = InferCtx {
                table: ctx.table,
                enum_registry: ctx.enum_registry,
                struct_registry: ctx.struct_registry,
                refined_decls: ctx.refined_decls,
                interface_registry: ctx.interface_registry,
                refines_registry: ctx.refines_registry,
                ret_ty: ctx.ret_ty,
                in_loop: true,
                deferred_lambdas: ctx.deferred_lambdas,
                path_conditions: ctx.path_conditions.clone(),
                post_conds: ctx.post_conds,
                inline_fns: ctx.inline_fns,
            };
            let mut typed_body = Vec::new();
            for expr in body {
                let typed = infer_expr(
                    &expr.node, &expr.span, env, &loop_ctx,
                    false, // body do loop nunca é tail_pos (loop retorna Unit)
                )?;
                typed_body.push(Spanned::new(typed, expr.span));
            }
            // Busca break recursivamente no body — pode estar dentro de match.
            let has_break = typed_body.iter().any(|s| contains_break(&s.node.kind));
            let loop_ty = if has_break {
                Ty::Unit
            } else {
                Ty::Var("_divergent".into())
            };
            (loop_ty, TypedExprKind::Loop { body: typed_body })
        }
        Expr::Break => {
            if !ctx.in_loop {
                return Err(MiddleError::TypeMismatch {
                    expected: "expressão (break só existe dentro de loop)".into(),
                    found: "Break".into(),
                    span: (*span).into(),
                });
            }
            (Ty::Unit, TypedExprKind::Break)
        }
        Expr::Continue => {
            if !ctx.in_loop {
                return Err(MiddleError::TypeMismatch {
                    expected: "expressão (continue só existe dentro de loop)".into(),
                    found: "Continue".into(),
                    span: (*span).into(),
                });
            }
            (Ty::Unit, TypedExprKind::Continue)
        }

        // ── `?` fail-fast — desugar para Match + Return ──
        Expr::Question(inner) => {
            return infer_question(inner, span, env, ctx, tail_pos);
        }
        // ── `|` fallback — desugar para Match (coalescência pura) ──
        Expr::PipeFallback { lhs, rhs } => {
            return infer_pipe_fallback(lhs, rhs, span, env, ctx);
        }
        // ── DotAccess (field access + index access) ──
        Expr::DotAccess { expr, index } => {
            return infer_dot_access(expr, index, span, env, ctx, tail_pos);
        }
        // ── Coleções — inferência delegada para collections.rs ──
        Expr::ListLit { elements } => {
            return super::collections::infer_list_lit(elements, span, env, ctx, tail_pos, hint);
        }
        Expr::ArrayLit { elements } => {
            return super::collections::infer_array_lit(elements, span, env, ctx, tail_pos);
        }
        Expr::DictLit { entries } => {
            return super::dict_set::infer_dict_lit(entries, span, env, ctx, tail_pos);
        }
        Expr::SetLit { elements } => {
            return super::dict_set::infer_set_lit(elements, span, env, ctx, tail_pos);
        }
        Expr::RangeLit {
            start,
            step,
            end,
            inclusive,
        } => {
            return super::collections::infer_range_lit(
                start, step, end, *inclusive, span, env, ctx, tail_pos,
            );
        }
        // ── ForIn e In ───────────────────────────────────────
        Expr::ForIn {
            var_name,
            iterable,
            body,
        } => {
            return super::collections::infer_for_in(
                var_name, iterable, body, span, env, ctx, tail_pos,
            );
        }
        Expr::In { item, collection } => {
            return super::collections::infer_in(item, collection, span, env, ctx, tail_pos);
        }

        // ── CSP — typeck em csp.rs ──
        Expr::ChannelSend { channel, value } => {
            return super::csp::infer_channel_send(channel, value, span, env, ctx, tail_pos);
        }
        Expr::ChannelRecv { channel, bind_name } => {
            return super::csp::infer_channel_recv(channel, bind_name, span, env, ctx, tail_pos);
        }
        Expr::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            return super::csp::infer_select(
                arms,
                timeout_ms,
                timeout_body,
                span,
                env,
                ctx,
                tail_pos,
            );
        }

        // ── `type!(expr)` — introspecção compile-time ──
        // ty = Text (sempre), kind = TypeOf { expr: typed_inner }.
        Expr::TypeOf { expr: inner } => {
            let typed_inner = infer_expr(&inner.node, &inner.span, env, ctx, false)?;
            (
                Ty::text(),
                TypedExprKind::TypeOf {
                    expr: Box::new(Spanned::new(typed_inner, inner.span)),
                },
            )
        }

        // ── Block: sequência de expressões —──────────────────
        // Usado em match arm body indentado com múltiplas statements.
        // Cada statement é inferida em sequência no mesmo escopo.
        // O resultado é a última expressão.
        Expr::Block { stmts } => {
            let mut typed_stmts: Vec<Spanned<TypedExpr>> = Vec::new();
            let mut last_ty = Ty::Unit;
            let n = stmts.len();
            for (i, stmt) in stmts.iter().enumerate() {
                // Propagar hint para o último statement (posição de cauda) e
                // para `let __result :=` (synthetic binding do Exit hook de
                // diretivas). Sem isto, o desugar do Exit hook envolve o body
                // em `let __result := <body>` que não recebe hint, produzindo
                // InferVar não-resolvida em chamadas recursivas com listas.
                let stmt_hint = if i + 1 == n {
                    hint
                } else if let Expr::Let { name, .. } = &stmt.node {
                    if name == "__result" { hint } else { None }
                } else {
                    None
                };
                let typed_stmt =
                    infer_expr_hinted(&stmt.node, &stmt.span, env, ctx, tail_pos, stmt_hint)?;
                last_ty = typed_stmt.ty.clone();
                typed_stmts.push(Spanned::new(typed_stmt, stmt.span));
            }
            (last_ty, TypedExprKind::Block { stmts: typed_stmts })
        }
    };

    // Deriva EscapeTarget de tail_pos + contexto (Action vs função pura/entry).
    let escape = if ctx.ret_ty.is_some() {
        // Dentro de Action: tail_pos = true → Caller, false → Local.
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        // Função pura / entry point: sem fiber_arena, tudo vai para a raiz.
        EscapeTarget::Caller
    };

    Ok(TypedExpr {
        span: *span,
        ty,
        tail_pos,
        escape,
        kind,
    })
}

/// Seleciona um overload de Action compatível com o hint de tipo esperado.
///
/// Segunda versão do PRD §12 (resolution por tipo esperado).
///
/// - Overload único: retorna-o diretamente (comportamento original).
/// - Múltiplos overloads + hint `Ty::Action(hint_params, hint_ret)`:
///   filtra por `match_score(hint_params, params)` compatível + `hint_ret == ret`.
///   Se exatamente um casa, retorna-o; senão, `None` (AmbiguousDispatch).
/// - Múltiplos overloads sem hint: `None` (AmbiguousDispatch).
fn select_action_overload<'a>(
    action_overloads: &[&'a kata_core::dispatch::OverloadInfo],
    hint: Option<&Ty>,
    ctx: &InferCtx,
) -> Option<&'a kata_core::dispatch::OverloadInfo> {
    // Overload único — sem ambiguidade.
    if action_overloads.len() == 1 {
        return Some(action_overloads[0]);
    }

    // Múltiplos overloads: tenta desambiguar pelo hint.
    let (hint_params, hint_ret) = match hint {
        Some(Ty::Action(params, ret)) => (params.as_slice(), ret.as_ref()),
        _ => return None, // Sem hint de Action → ambíguo.
    };

    let compatibles: Vec<_> = action_overloads
        .iter()
        .filter(|o| {
            // Aridade deve bater.
            if o.params.len() != hint_params.len() {
                return false;
            }
            // match_score compara params do hint (como "args") vs params do overload.
            let score = match_score(hint_params, &o.params, ctx.interface_registry);
            if !score.is_compatible(hint_params.len()) {
                return false;
            }
            // Retorno deve ser igual.
            &o.ret == hint_ret
        })
        .copied()
        .collect();

    match compatibles.len() {
        1 => Some(compatibles[0]),
        _ => None, // 0 ou >1 → ambíguo.
    }
}

/// Gera sugestões "você quis dizer X?" para um nome não-vinculado.
///
/// Usa distância de Levenshtein simples (conta edições) para encontrar
/// nomes no escopo que são parecidos com o nome digitado. Retorna
/// `None` se nenhuma sugestão for boa o suficiente.
pub(super) fn suggest_similar<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<String> {
    /// Distância de Levenshtein — número mínimo de edições
    /// (inserção, deleção, substituição) para transformar a em b.
    fn levenshtein(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let (m, n) = (a.len(), b.len());
        if m == 0 {
            return n;
        }
        if n == 0 {
            return m;
        }
        let mut prev: Vec<usize> = (0..=n).collect();
        let mut curr: Vec<usize> = vec![0; n + 1];
        for i in 1..=m {
            curr[0] = i;
            for j in 1..=n {
                let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
                curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
            }
            std::mem::swap(&mut prev, &mut curr);
        }
        prev[n]
    }

    let max_dist = (name.len() / 3).max(2); // tolerância proporcional ao tamanho
    let mut best: Vec<(usize, &str)> = Vec::new();
    for candidate in candidates {
        let dist = levenshtein(name, candidate);
        if dist <= max_dist && dist > 0 {
            best.push((dist, candidate));
        }
    }
    if best.is_empty() {
        None
    } else {
        // Ordenar por (distância, nome) para output determinístico
        // (estável entre runs — importante para snapshots).
        best.sort_by(|(d1, n1), (d2, n2)| d1.cmp(d2).then(n1.cmp(n2)));
        best.truncate(3);
        Some(format!(
            "você quis dizer {}?",
            best.iter()
                .map(|(_, s)| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}
