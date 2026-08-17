//! Síntese de `show` para `List::A` — extraído de `show_synthesis.rs`.
//!
//! Gera duas TypedFunction genéricas mutuamente recursivas:
//!
//! - `__kata_show__List :: List::A => Text`
//! - `__kata_show__List_rest :: List::A => Text`

use kata_ast::Span;
use kata_ast::Spanned;
use kata_core::dispatch::{DispatchTable, OverloadInfo};
use kata_core::escape::EscapeTarget;
use kata_core::interface_registry::{ImplEntry, ImplMethodInfo, InterfaceRegistry};
use kata_core::ty::Ty;

use crate::typed::{
    TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause, TypedMatchArm, TypedPattern,
};

use super::show_synthesis_helpers::{repr_expr, show_call, string_concat, text_lit};

/// Sintetiza `show` para `List::A` — lista persistente (Cons/Nil).
///
/// Gera duas TypedFunction genéricas mutuamente recursivas:
///
/// - `__kata_show__List :: List::A => Text`
///   - `Cons(h, t)` → `string_concat("[", string_concat(show h, __kata_show__List_rest t))`
///   - `Nil` → `"[]"`
///
/// - `__kata_show__List_rest :: List::A => Text`
///   - `Cons(h, t)` → `string_concat(", ", string_concat(show h, __kata_show__List_rest t))`
///   - `Nil` → `"]"`
///
/// Registra:
/// - `show :: List::A => Text @ffi("__kata_show__List")` no DispatchTable (genérico, type_params: ["A"])
/// - `__kata_show__List_rest :: List::A => Text @ffi("__kata_show__List_rest")` no DispatchTable (genérico, type_params: ["A"])
/// - `List implements SHOW` no InterfaceRegistry (type_params: ["A"])
///
/// O monomorphizador instancia ambas quando um call site `show` com arg
/// `List(Int)` é encontrado, e propaga a instância de `__kata_show__List_rest`
/// via `rewrite_typed_expr` dentro do body de `__kata_show__List`.
pub(crate) fn synthesize_list_show_functions(
    dispatch_table: &mut DispatchTable,
    interface_registry: &mut InterfaceRegistry,
) -> Vec<TypedFunction> {
    let type_param = "A";
    let elem_ty = Ty::Var(type_param.to_string());
    let list_ty = Ty::List(Box::new(elem_ty.clone()));
    let ret_ty = Ty::text();

    // ── Registra overload `show :: List::A => Text` no DispatchTable ──
    dispatch_table.insert(OverloadInfo {
        name: "show".to_string(),
        params: vec![list_ty.clone()],
        ret: ret_ty.clone(),
        ffi_symbol: Some("__kata_show__List".to_string()),
        is_action: false,
        is_generic: true,
        is_constructor: false,
        associative_neutral: None,
        type_params: vec![type_param.to_string()],
        substitutions: None,
        param_names: vec![],
        param_defaults: vec![],
    });

    // ── Registra overload `__kata_show__List_rest :: List::A => Text` ──
    //    Nome único no DispatchTable para que o monomorphizador descubra e
    //    instancie via `instantiate_generic_closure`.
    let rest_name = "__kata_show__List_rest";
    dispatch_table.insert(OverloadInfo {
        name: rest_name.to_string(),
        params: vec![list_ty.clone()],
        ret: ret_ty.clone(),
        ffi_symbol: Some(rest_name.to_string()),
        is_action: false,
        is_generic: true,
        is_constructor: false,
        associative_neutral: None,
        type_params: vec![type_param.to_string()],
        substitutions: None,
        param_names: vec![],
        param_defaults: vec![],
    });

    // ── Registra `List implements SHOW` no InterfaceRegistry ──
    interface_registry
        .register_impl(ImplEntry {
            origin: "__synthesis".to_string(),
            type_name: "List".to_string(),
            type_params: vec![type_param.to_string()],
            interface_name: "SHOW".to_string(),
            iface_params: vec![],
            methods: vec![ImplMethodInfo {
                name: "show".to_string(),
                params: vec![list_ty.clone()],
                ret: ret_ty.clone(),
                ffi_symbol: Some("__kata_show__List".to_string()),
            }],
        })
        .ok();

    // ── Constrói as duas TypedFunction ──
    let show_list =
        build_list_show_func("__kata_show__List", &list_ty, &ret_ty, &elem_ty, "[", "[]");
    let show_list_rest = build_list_show_func(rest_name, &list_ty, &ret_ty, &elem_ty, ", ", "]");

    vec![show_list, show_list_rest]
}

/// Constrói uma TypedFunction para show de List (principal ou rest).
///
/// - `func_name`: nome mangled (`__kata_show__List` ou `__kata_show__List_rest`)
/// - `list_ty`: `List(Var("A"))` — tipo do parâmetro
/// - `ret_ty`: `Text`
/// - `elem_ty`: `Var("A")` — tipo do elemento (para `show h`)
/// - `sep`: separador antes do head (`"["` para principal, `", "` para rest)
/// - `nil_body`: texto do caso Nil (`"[]"` para principal, `"]"` para rest)
fn build_list_show_func(
    func_name: &str,
    list_ty: &Ty,
    ret_ty: &Ty,
    elem_ty: &Ty,
    sep: &str,
    nil_body: &str,
) -> TypedFunction {
    let pattern = Spanned::new(
        TypedPattern::Ident {
            name: "__self".to_string(),
            ty: list_ty.clone(),
        },
        Span::synthetic(),
    );

    let body = build_list_show_body(list_ty, elem_ty, sep, nil_body);

    TypedFunction {
        name: func_name.to_string(),
        param_types: vec![list_ty.clone()],
        ret_ty: ret_ty.clone(),
        clauses: vec![TypedLambdaClause {
            patterns: vec![pattern],
            body: Spanned::new(body, Span::synthetic()),
            guards: Vec::new(),
            with_bindings: Vec::new(),
        }],
        cache_spec: None,
        timer_spec: None,
    }
}

/// Constrói o body de `show` para List como `Match __self`:
///
/// - `Cons(h, t)`: `string_concat(sep, string_concat(show h, __kata_show__List_rest t))`
/// - `Nil`: `TextLit(nil_body)`
///
/// Ambas as funções (`__kata_show__List` e `__kata_show__List_rest`) chamam
/// `__kata_show__List_rest` no braço Cons — a principal invoca o rest para
/// processar a cauda, e o rest invoca a si mesmo recursivamente.
fn build_list_show_body(list_ty: &Ty, elem_ty: &Ty, sep: &str, nil_body: &str) -> TypedExpr {
    // Scrutinee: __self
    let scrutinee = TypedExpr {
        span: Span::synthetic(),
        ty: list_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "__self".to_string(),
        },
    };
    let scrutinee = Spanned::new(scrutinee, Span::synthetic());

    // ── Braço Cons(h, t) ──
    let head_pat = Spanned::new(
        TypedPattern::Ident {
            name: "h".to_string(),
            ty: elem_ty.clone(),
        },
        Span::synthetic(),
    );
    let tail_pat = Spanned::new(
        TypedPattern::Ident {
            name: "t".to_string(),
            ty: list_ty.clone(),
        },
        Span::synthetic(),
    );
    let cons_pat = TypedPattern::Cons {
        head: Box::new(head_pat),
        tail: Box::new(tail_pat),
    };

    // `h` como expr
    let h_expr = TypedExpr {
        span: Span::synthetic(),
        ty: elem_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "h".to_string(),
        },
    };
    let h_spanned = Spanned::new(h_expr, Span::synthetic());

    // `repr h` — despacha via repr_expr (cita Text, delega para show_expr
    // nos demais). elem_ty é Ty::Var("A"); o monomorphizador resolve via Layer 5.
    let show_h = repr_expr(h_spanned, elem_ty);

    // `t` como expr
    let t_expr = TypedExpr {
        span: Span::synthetic(),
        ty: list_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "t".to_string(),
        },
    };
    let t_spanned = Spanned::new(t_expr, Span::synthetic());

    // `<rest_call> t` — chama `__kata_show__List_rest` (ambas as funções
    // chamam __kata_show__List_rest: a principal chama o rest, o rest
    // chama a si mesmo recursivamente)
    let rest_call = show_call(t_spanned, "List_rest".to_string(), list_ty);

    // body do Cons: `string_concat(sep, string_concat(show h, rest_call t))`
    let cons_body = string_concat(text_lit(sep.to_string()), string_concat(show_h, rest_call));

    let cons_arm = TypedMatchArm {
        pattern: Some(Spanned::new(cons_pat, Span::synthetic())),
        guard: None,
        body: cons_body,
    };

    // ── Braço Nil ──
    let nil_arm = TypedMatchArm {
        pattern: Some(Spanned::new(TypedPattern::Nil, Span::synthetic())),
        guard: None,
        body: text_lit(nil_body.to_string()),
    };

    // Match sobre __self
    TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: true,
        escape: EscapeTarget::Caller,
        kind: TypedExprKind::Match {
            scrutinee: Box::new(scrutinee),
            arms: vec![cons_arm, nil_arm],
        },
    }
}
