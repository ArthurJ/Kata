//! Fio 6 Fase 3 — Síntese de funções predicado e smart constructors falíveis.
//!
//! Para cada `RefinedDeclInfo` (declarado com `data (Int, > _ 0) as PositiveInt`):
//!
//! 1. **Funções predicado** — `__pred_PositiveInt_0 :: Int => Boolean`.
//!    Body = predicado com `Hole` substituído por `Ident("x")`, inferido
//!    normalmente (despacha operadores do prelude como `>`, `<`, etc.).
//!
//! 2. **Smart constructor falível** — `PositiveInt :: Int => Result::(PositiveInt, Text)`.
//!    Body = match aninhado sobre os predicados:
//!    ```text
//!    match __pred_0(v) {
//!      Boolean::True: match __pred_1(v) {
//!        Boolean::True: Result::Ok(v)
//!        Boolean::False: Result::Err("predicado 1 falhou em TypeName")
//!      }
//!      Boolean::False: Result::Err("predicado 0 falhou em TypeName")
//!    }
//!    ```
//!    Construído manualmente (não via `infer_expr`) para evitar problemas
//!    de unificação de tipos `Result` entre guards.

use kata_ast::{Expr, Span, Spanned};
use kata_core::dispatch::{DispatchTable, OverloadInfo};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_resolution::RefinedDeclInfo;

use crate::desugar;
use crate::typed::{
    Effect, TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause, TypedMatchArm, TypedPattern,
};

use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;

/// Sintetiza funções predicado e smart constructors falíveis para tipos refinados.
///
/// Retorna `Vec<TypedFunction>` com todas as funções sintetizadas (predicados +
/// construtores falíveis). Registra overloads no `dispatch_table`.
pub(crate) fn synthesize_refined(
    refined_decls: &[RefinedDeclInfo],
    enum_registry: &kata_core::enum_registry::EnumRegistry,
    struct_registry: &kata_core::struct_registry::StructRegistry,
    type_env: &TypeEnv,
    dispatch_table: &mut DispatchTable,
) -> InferResult<Vec<TypedFunction>> {
    if refined_decls.is_empty() {
        return Ok(Vec::new());
    }

    // ── Passo 1: registra assinaturas no DispatchTable ──
    for decl in refined_decls {
        let pred_names = struct_registry
            .get(&decl.name)
            .and_then(|si| si.predicates.as_ref())
            .expect("refined decl deve ter predicates no StructRegistry");

        // Registra cada função predicado: __pred_TypeName_N :: base_ty => Boolean
        for pred_name in pred_names {
            dispatch_table.insert(OverloadInfo {
                name: pred_name.clone(),
                params: vec![decl.base_ty.clone()],
                ret: Ty::boolean(),
                ffi_symbol: None,
                is_action: false,
                is_generic: false,
                is_constructor: false,
                associative_neutral: None,
            });
        }

        // Registra smart constructor falível: TypeName :: base_ty => Result::(TypeName, Text)
        let ret_ty = Ty::Generic(
            "Result".into(),
            vec![Ty::Struct(decl.name.clone()), Ty::text()],
        );
        dispatch_table.insert(OverloadInfo {
            name: decl.name.clone(),
            params: vec![decl.base_ty.clone()],
            ret: ret_ty,
            ffi_symbol: None,
            is_action: false,
            is_generic: false,
            is_constructor: true,
            associative_neutral: None,
        });
    }

    // ── Passo 2: sintetiza TypedFunctions ──
    // Cria InferCtx com o DispatchTable já populado (borrow imutável).
    // Construtores refined não usam interfaces — registry vazio.
    let empty_iface_reg = kata_core::interface_registry::InterfaceRegistry::new();
    let ctx = InferCtx {
        table: &*dispatch_table,
        enum_registry,
        struct_registry,
        refined_decls: &[],
        interface_registry: &empty_iface_reg,
        ret_ty: None,
        in_loop: false,
    };

    let mut functions = Vec::new();

    for decl in refined_decls {
        let pred_names = struct_registry
            .get(&decl.name)
            .and_then(|si| si.predicates.as_ref())
            .expect("refined decl deve ter predicates");

        // ── 2a. Sintetiza funções predicado ──
        for (i, pred_name) in pred_names.iter().enumerate() {
            let pred_expr = &decl.predicates[i];

            // Substitui Hole por Ident("x") no predicado
            let substituted = substitute_hole(pred_expr, "x");
            let desugared = desugar::desugar(&substituted);

            // Cria TypeEnv com x: base_ty
            let mut pred_env = type_env.clone();
            pred_env.define("x", decl.base_ty.clone());

            // Infere o body do predicado (despacha operadores via prelude)
            let typed_body = infer_expr(
                &desugared.node,
                &desugared.span,
                &mut pred_env,
                &ctx,
                true, // tail_pos
            )?;

            // Monta a TypedFunction do predicado
            let pattern = Spanned::new(
                TypedPattern::Ident {
                    name: "x".into(),
                    ty: decl.base_ty.clone(),
                },
                Span::synthetic(),
            );

            functions.push(TypedFunction {
                name: pred_name.clone(),
                param_types: vec![decl.base_ty.clone()],
                ret_ty: Ty::boolean(),
                clauses: vec![TypedLambdaClause {
                    patterns: vec![pattern],
                    body: Spanned::new(typed_body, desugared.span),
                    guards: Vec::new(),
                    with_bindings: Vec::new(),
                }],
            });
        }

        // ── 2b. Sintetiza smart constructor falível ──
        let result_ty = Ty::Generic(
            "Result".into(),
            vec![Ty::Struct(decl.name.clone()), Ty::text()],
        );

        // Constrói as chamadas dos predicados: __pred_N(v) para cada predicado
        let pred_calls: Vec<Spanned<TypedExpr>> = pred_names
            .iter()
            .map(|pn| build_pred_call(pn, "v", &decl.base_ty))
            .collect();

        // Constrói Result::Ok(v) — o body do nível mais profundo do match
        let ok_body = build_result_ok("v", &decl.base_ty, &decl.name, &result_ty);

        // Constrói Result::Err para cada predicado que falha.
        // Mensagem dinâmica: "{v} falhou no predicado {pred_str} na construção do {type_name}"
        let err_bodies: Vec<Spanned<TypedExpr>> = pred_names
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let pred_str = expr_to_string(&decl.predicates[i]);
                build_result_err("v", &decl.base_ty, &pred_str, &decl.name, &result_ty)
            })
            .collect();

        // Constrói o match aninhado
        let body = if pred_calls.is_empty() {
            // Sem predicados? Não deveria acontecer (refined sem predicados
            // não é refined), mas defensivo: sempre Ok.
            ok_body
        } else {
            build_nested_match(&pred_calls, &err_bodies, ok_body, &result_ty)
        };

        let pattern = Spanned::new(
            TypedPattern::Ident {
                name: "v".into(),
                ty: decl.base_ty.clone(),
            },
            Span::synthetic(),
        );

        functions.push(TypedFunction {
            name: decl.name.clone(),
            param_types: vec![decl.base_ty.clone()],
            ret_ty: result_ty,
            clauses: vec![TypedLambdaClause {
                patterns: vec![pattern],
                body,
                guards: Vec::new(),
                with_bindings: Vec::new(),
            }],
        });
    }

    Ok(functions)
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Substitui `Expr::Hole` por `Expr::Ident { name: var_name }` recursivamente.
pub(crate) fn substitute_hole(expr: &Spanned<Expr>, var_name: &str) -> Spanned<Expr> {
    let new_node = match &expr.node {
        Expr::Hole => Expr::Ident {
            name: var_name.into(),
        },
        Expr::Apply { callee, args } => Expr::Apply {
            callee: Box::new(substitute_hole(callee, var_name)),
            args: args.iter().map(|a| substitute_hole(a, var_name)).collect(),
        },
        Expr::TypeAscription { expr: inner, ty } => Expr::TypeAscription {
            expr: Box::new(substitute_hole(inner, var_name)),
            ty: ty.clone(),
        },
        Expr::Grouping { inner } => Expr::Grouping {
            inner: Box::new(substitute_hole(inner, var_name)),
        },
        Expr::Tuple { elements } => Expr::Tuple {
            elements: elements
                .iter()
                .map(|e| substitute_hole(e, var_name))
                .collect(),
        },
        other => other.clone(),
    };
    Spanned::new(new_node, expr.span)
}

/// Constrói uma chamada de predicado: `__pred_Name_N(v)` como `TypedExpr`.
pub(crate) fn build_pred_call(pred_name: &str, var_name: &str, base_ty: &Ty) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![base_ty.clone()], Box::new(Ty::boolean())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: pred_name.into(),
        },
    };
    let arg = TypedExpr {
        span: Span::synthetic(),
        ty: base_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: var_name.into(),
        },
    };
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::boolean(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![Spanned::new(arg, Span::synthetic())],
                ffi_symbol: None,
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `Result::Ok(v)` como `TypedExpr` com tipo `result_ty`.
fn build_result_ok(
    var_name: &str,
    base_ty: &Ty,
    _type_name: &str,
    result_ty: &Ty,
) -> Spanned<TypedExpr> {
    let payload = TypedExpr {
        span: Span::synthetic(),
        ty: base_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: var_name.into(),
        },
    };
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: result_ty.clone(),
            tail_pos: true,
            escape: EscapeTarget::Caller,
            effect: Effect::Puro,
            kind: TypedExprKind::VariantConstruct {
                enum_name: "Result".into(),
                variant: "Ok".into(),
                payload: Box::new(Spanned::new(payload, Span::synthetic())),
                tag: 0,
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `Result::Err(msg)` onde `msg = StringConcat(Show(v), TextLit(...))`.
///
/// A mensagem é: `"{v} falhou no predicado {pred_str} na construção do {type_name}"`.
/// `Show(v)` despacha via FFI conforme o tipo base (Int→bi_show, Float→float_to_text, Rational→rat_show).
fn build_result_err(
    var_name: &str,
    base_ty: &Ty,
    pred_str: &str,
    type_name: &str,
    result_ty: &Ty,
) -> Spanned<TypedExpr> {
    let suffix = format!(" falhou no predicado {pred_str} na construção do {type_name}");

    // Show(v) — chama o FFI apropriado para converter v em Text.
    let shown = build_show_call(var_name, base_ty);

    // TextLit do sufixo.
    let suffix_lit = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::TextLit { text: suffix },
    };

    // StringConcat(Show(v), suffix_lit) → Text.
    let concat = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(
                TypedExpr {
                    span: Span::synthetic(),
                    ty: Ty::Function(vec![Ty::text(), Ty::text()], Box::new(Ty::text())),
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    effect: Effect::Puro,
                    kind: TypedExprKind::Ident {
                        name: "kata_rt_string_concat".into(),
                    },
                },
                Span::synthetic(),
            )),
            args: vec![shown, Spanned::new(suffix_lit, Span::synthetic())],
            ffi_symbol: Some("kata_rt_string_concat".into()),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: result_ty.clone(),
            tail_pos: true,
            escape: EscapeTarget::Caller,
            effect: Effect::Puro,
            kind: TypedExprKind::VariantConstruct {
                enum_name: "Result".into(),
                variant: "Err".into(),
                payload: Box::new(Spanned::new(concat, Span::synthetic())),
                tag: 1,
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `Show(v)` — converte o valor `v` do tipo base para Text via FFI.
///
/// Int → `kata_rt_bi_show`, Float → `kata_rt_float_to_text`, Rational → `kata_rt_rat_show`.
fn build_show_call(var_name: &str, base_ty: &Ty) -> Spanned<TypedExpr> {
    let (ffi_name, param_clif) = match base_ty {
        Ty::Prim(PrimTy::Int) => ("kata_rt_bi_show", false),
        Ty::Prim(PrimTy::Float) => ("kata_rt_float_to_text", true),
        Ty::Prim(PrimTy::Rational) => ("kata_rt_rat_show", false),
        _ => ("kata_rt_bi_show", false), // fallback defensivo
    };
    let _ = param_clif;

    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![base_ty.clone()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: ffi_name.into(),
        },
    };
    let arg = TypedExpr {
        span: Span::synthetic(),
        ty: base_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: var_name.into(),
        },
    };
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![Spanned::new(arg, Span::synthetic())],
                ffi_symbol: Some(ffi_name.into()),
            },
        },
        Span::synthetic(),
    )
}

/// Stringifica um predicado AST para a mensagem de erro.
///
/// `> _ 0` → `"> _ 0"`, `<= _ 100` → `"<= _ 100"`.
pub(crate) fn expr_to_string(expr: &Spanned<Expr>) -> String {
    match &expr.node {
        Expr::Ident { name } => name.clone(),
        Expr::Hole => "_".into(),
        Expr::IntLit { text } => text.clone(),
        Expr::FloatLit { text } => text.clone(),
        Expr::TextLit { text } => format!("\"{text}\""),
        Expr::Apply { callee, args } => {
            let args_str = args
                .iter()
                .map(expr_to_string)
                .collect::<Vec<_>>()
                .join(" ");
            format!("{} {args_str}", expr_to_string(callee))
        }
        Expr::Grouping { inner } => expr_to_string(inner),
        _ => "{?}".into(),
    }
}

/// Constrói um match aninhado sobre os predicados.
///
/// `pred_calls[0]` é o scrutinee do match mais externo.
/// Se True → recursa para `pred_calls[1..]`.
/// Se False → `err_bodies[0]`.
///
/// No nível mais profundo (sem mais predicados) → `ok_body`.
fn build_nested_match(
    pred_calls: &[Spanned<TypedExpr>],
    err_bodies: &[Spanned<TypedExpr>],
    ok_body: Spanned<TypedExpr>,
    result_ty: &Ty,
) -> Spanned<TypedExpr> {
    if pred_calls.is_empty() {
        return ok_body;
    }

    let scrutinee = pred_calls[0].clone();
    let err_body = err_bodies[0].clone();

    let inner = build_nested_match(&pred_calls[1..], &err_bodies[1..], ok_body, result_ty);

    // Arm True: recursa para o próximo predicado
    let true_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Boolean".into(),
                variant: "True".into(),
                sub_patterns: None,
                tag: 0,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: inner,
    };

    // Arm False: erro
    let false_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Boolean".into(),
                variant: "False".into(),
                sub_patterns: None,
                tag: 1,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: err_body,
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: result_ty.clone(),
            tail_pos: true,
            escape: EscapeTarget::Caller,
            effect: Effect::Puro,
            kind: TypedExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms: vec![true_arm, false_arm],
            },
        },
        Span::synthetic(),
    )
}

// `synthesize_enum_pred` e `build_variant_construct` extraídos para
// `constructors_enum_pred.rs`. Helpers compartilhados (`substitute_hole`,
// `build_pred_call`, `expr_to_string`) são `pub(crate)` aqui.
