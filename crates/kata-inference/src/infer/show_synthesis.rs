//! Síntese de `show` — implementação auto-sintetizada de SHOW para
//! `data` (structs) com campos e `enum` (genérico ou não).
//!
//! Para cada `data Nome (campos)`, o typeck sintetiza `show :: Nome => Text`
//! no DispatchTable + `TypedFunction` com body que constrói a string
//! `"Nome(field0, field1, ...)"` via `kata_rt_string_concat` (FFI binário).
//! Registra também `Nome implements SHOW` no `InterfaceRegistry`.
//!
//! Para cada `enum Nome { variantes }`, sintetiza `show :: Nome => Text`
//! com body `Match` sobre `__self` — um braço por variante:
//! - Variante unitária → `TextLit("VariantName")`
//! - Variante com payload → `string_concat("VariantName(", show payload, ")")`
//!
//! Enums genéricos (ex: `Result::(T, E)`) geram `show` genérico — o body
//! contém `show v` onde `v :: Ty::Var("T")`, resolvido pelo monomorphizador
//! ao instanciar. A asserção implícita "todo Ty::Var implementa SHOW" é
//! garantida pela invariant de que todo tipo concreto implementará SHOW.

use kata_ast::{Span, Spanned};
use kata_core::dispatch::{DispatchTable, OverloadInfo};
use kata_core::enum_registry::EnumRegistry;
use kata_core::escape::EscapeTarget;
use kata_core::interface_registry::{ImplEntry, ImplMethodInfo, InterfaceRegistry};
use kata_core::struct_registry::StructRegistry;
use kata_core::ty::{PrimTy, Ty};

use crate::typed::{
    Effect, TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause, TypedMatchArm, TypedPattern,
};

use super::show_synthesis_helpers::{
    ffi_call1, field_access_expr, show_call, show_expr, string_concat, text_lit,
};

/// Verifica se um tipo já tem implementação manual do método `show` (via
/// qualquer interface). Respeita orphan rule: o impl registrado no
/// InterfaceRegistry está no mesmo módulo que o tipo, então não há
/// violação. Se há implementação manual, a síntese é skipada.
fn has_manual_show(interface_registry: &InterfaceRegistry, type_name: &str) -> bool {
    interface_registry
        .get_impls_for_type(type_name)
        .iter()
        .any(|impl_entry| impl_entry.methods.iter().any(|m| m.name == "show"))
}

/// Síntese de funções `show` para structs com campos e todos os enums.
///
/// Retorna `Vec<TypedFunction>` que deve ser adicionada a
/// `typed_module.functions`. Os overloads já são registrados no
/// `dispatch_table` e os impls no `interface_registry`.
pub(crate) fn synthesize_show_functions(
    struct_registry: &StructRegistry,
    enum_registry: &EnumRegistry,
    dispatch_table: &mut DispatchTable,
    interface_registry: &mut InterfaceRegistry,
) -> Vec<TypedFunction> {
    let mut show_functions = Vec::new();

    // ── Structs ──────────────────────────────────────────────
    for struct_name in struct_registry.names() {
        let struct_info = struct_registry
            .get(struct_name)
            .expect("struct_name veio de struct_registry.names()");

        // Aliases não ganham show próprio — usam o show do target.
        if struct_info.alias_of.is_some() {
            continue;
        }

        // Structs sem campos não ganham show (não há o que mostrar).
        if struct_info.fields.is_empty() {
            continue;
        }

        // Se o tipo já tem implementação manual do método `show` (via qualquer
        // interface, respeitando orphan rule — o impl está no mesmo módulo que
        // o tipo), não sintetiza. A implementação manual tem prioridade.
        if has_manual_show(interface_registry, struct_name) {
            continue;
        }

        let ret_ty = Ty::text();
        let param_ty = Ty::Struct(struct_name.to_string());
        let mangled = format!("__kata_show__{struct_name}");

        // Registra overload `show :: Struct => Text` no DispatchTable.
        dispatch_table.insert(OverloadInfo {
            name: "show".to_string(),
            params: vec![param_ty.clone()],
            ret: ret_ty.clone(),
            ffi_symbol: Some(mangled.clone()),
            is_action: false,
            is_generic: false,
            is_constructor: false,
            associative_neutral: None,
            type_params: vec![],
            substitutions: None,
        });

        // Registra `Struct implements SHOW` no InterfaceRegistry.
        interface_registry
            .register_impl(ImplEntry {
                type_name: struct_name.to_string(),
                type_params: vec![],
                interface_name: "SHOW".to_string(),
                iface_params: vec![],
                methods: vec![ImplMethodInfo {
                    name: "show".to_string(),
                    params: vec![param_ty.clone()],
                    ret: ret_ty.clone(),
                    ffi_symbol: Some(mangled.clone()),
                }],
            })
            .ok();

        // Pattern: `__self` : Struct
        let pattern = Spanned::new(
            TypedPattern::Ident {
                name: "__self".to_string(),
                ty: param_ty.clone(),
            },
            Span::synthetic(),
        );

        // Constrói o body: string_concat aninhado
        let body = build_struct_show_body(struct_name, &struct_info.fields, struct_registry);

        show_functions.push(TypedFunction {
            name: mangled,
            param_types: vec![param_ty],
            ret_ty,
            clauses: vec![TypedLambdaClause {
                patterns: vec![pattern],
                body: Spanned::new(body, Span::synthetic()),
                guards: Vec::new(),
                with_bindings: Vec::new(),
            }],
            log: None,
        });
    }

    // ── Enums ───────────────────────────────────────────────
    for enum_name in enum_registry.names() {
        let variants = match enum_registry.all_variants(enum_name) {
            Some(vs) => vs,
            None => continue,
        };

        // Skipar enums sem variantes (não deveria acontecer, mas defensivo).
        if variants.is_empty() {
            continue;
        }

        // Se o enum já tem implementação manual do método `show`, não sintetiza.
        if has_manual_show(interface_registry, enum_name) {
            continue;
        }

        let is_generic = enum_registry.is_generic(enum_name);
        let type_params: Vec<String> = enum_registry
            .type_params_of(enum_name)
            .map(|s| s.to_vec())
            .unwrap_or_default();

        let ret_ty = Ty::text();
        // Para enums genéricos, o param é `Ty::Generic("Result", [Var("T"), Var("E")])`
        // para que o monomorphizador instancie T e E. Para enums não-genéricos,
        // é `Ty::Sum("Enum")` direto.
        let param_ty = if is_generic {
            Ty::Generic(
                enum_name.to_string(),
                type_params.iter().map(|p| Ty::Var(p.clone())).collect(),
            )
        } else {
            Ty::Sum(enum_name.to_string())
        };
        let mangled = format!("__kata_show__{enum_name}");

        // Registra overload `show :: Enum => Text` no DispatchTable.
        dispatch_table.insert(OverloadInfo {
            name: "show".to_string(),
            params: vec![param_ty.clone()],
            ret: ret_ty.clone(),
            ffi_symbol: Some(mangled.clone()),
            is_action: false,
            is_generic,
            is_constructor: false,
            associative_neutral: None,
            type_params: type_params.clone(),
            substitutions: None,
        });

        // Registra `Enum implements SHOW` no InterfaceRegistry.
        interface_registry
            .register_impl(ImplEntry {
                type_name: enum_name.to_string(),
                type_params: type_params.clone(),
                interface_name: "SHOW".to_string(),
                iface_params: vec![],
                methods: vec![ImplMethodInfo {
                    name: "show".to_string(),
                    params: vec![param_ty.clone()],
                    ret: ret_ty.clone(),
                    ffi_symbol: Some(mangled.clone()),
                }],
            })
            .ok();

        // Pattern: `__self` : Enum
        let pattern = Spanned::new(
            TypedPattern::Ident {
                name: "__self".to_string(),
                ty: param_ty.clone(),
            },
            Span::synthetic(),
        );

        // Constrói o body: Match sobre __self, um braço por variante.
        let body = build_enum_show_body(enum_name, variants, enum_registry);

        show_functions.push(TypedFunction {
            name: mangled,
            param_types: vec![param_ty],
            ret_ty,
            clauses: vec![TypedLambdaClause {
                patterns: vec![pattern],
                body: Spanned::new(body, Span::synthetic()),
                guards: Vec::new(),
                with_bindings: Vec::new(),
            }],
            log: None,
        });
    }

    show_functions
}

// ════════════════════════════════════════════════════════════════
// Structs
// ════════════════════════════════════════════════════════════════

/// Constrói o body de `show` para struct como árvore aninhada de
/// `string_concat`. `Nome(f0, f1, ..., fN)` =
///   concat("Nome(", concat(f0_show, concat(", ", concat(f1_show, ... ")"))))
fn build_struct_show_body(
    struct_name: &str,
    fields: &[kata_core::struct_registry::FieldInfo],
    struct_registry: &StructRegistry,
) -> TypedExpr {
    let mut parts: Vec<Spanned<TypedExpr>> = Vec::new();

    parts.push(text_lit(format!("{struct_name}(")));

    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            parts.push(text_lit(", ".to_string()));
        }
        parts.push(field_show(field, i, struct_registry));
    }

    parts.push(text_lit(")".to_string()));

    let result = parts.into_iter().reduce(string_concat);
    let body = result.expect("show body tem pelo menos 2 parts");

    TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: true,
        escape: EscapeTarget::Ancestor(0),
        effect: Effect::Puro,
        kind: body.node.kind,
    }
}

/// Produz a representação Text de um campo de struct.
///
/// - `Text`: identity (FieldAccess direto)
/// - `Int`: `kata_rt_int_to_text(field_access)`
/// - `Rational`: `kata_rt_rat_show(field_access)`
/// - `Float`: `kata_rt_float_to_text(field_access)`
/// - `Boolean` (Sum): `__kata_show__Boolean(field_access)` — chama o show
///   sintetizado para Boolean (variantes True/False → "True"/"False")
/// - `Struct` aninhado: chamada recursiva a `show` (call direto via mangled)
/// - `Sum` (enum): chamada a `__kata_show__{Enum}` (recursão entre sintetizados)
/// - Outros: fallback `kata_rt_int_to_text`
fn field_show(
    field: &kata_core::struct_registry::FieldInfo,
    field_index: usize,
    _struct_registry: &StructRegistry,
) -> Spanned<TypedExpr> {
    let field_access = field_access_expr(field_index, &field.ty);

    match &field.ty {
        Ty::Prim(PrimTy::Text) => field_access,
        Ty::Prim(PrimTy::Int) => ffi_call1("kata_rt_int_to_text", field_access, Ty::text()),
        Ty::Prim(PrimTy::Rational) => ffi_call1("kata_rt_rat_show", field_access, Ty::text()),
        Ty::Prim(PrimTy::Float) => ffi_call1("kata_rt_float_to_text", field_access, Ty::text()),
        Ty::Sum(name) => {
            // Enum (incluindo Boolean) — chama `__kata_show__{Enum}` mangled.
            show_call(field_access, name.clone(), &field.ty)
        }
        Ty::Struct(name) => {
            // Struct aninhado — chama `__kata_show__{Struct}` mangled.
            show_call(field_access, name.clone(), &field.ty)
        }
        Ty::List(_) => {
            // List — chama `__kata_show__List` mangled (genérico, instanciado
            // pelo monomorphizador para o tipo concreto do elemento).
            show_call(field_access, "List".to_string(), &field.ty)
        }
        _ => {
            // Fallback: int_to_text (mostra como número — melhor que crash)
            ffi_call1("kata_rt_int_to_text", field_access, Ty::text())
        }
    }
}

// ════════════════════════════════════════════════════════════════
// Enums
// ════════════════════════════════════════════════════════════════

/// Constrói o body de `show` para enum como um `Match` sobre `__self`,
/// com um braço por variante.
///
/// - Variante unitária → `TextLit("VariantName")`
/// - Variante com payload → `string_concat("VariantName(", show payload, ")")`
///
/// O pattern de cada braço é `TypedPattern::Variant { enum_name, variant,
/// sub_patterns, tag }`. Para variante com payload, o sub-pattern é
/// `Ident { name: "v", ty: payload_ty }` — o binding `v` é acessível
/// no body do braço (o codegen faz `def_var` no `test_single_pattern`).
fn build_enum_show_body(
    enum_name: &str,
    variants: &[kata_core::enum_registry::VariantInfo],
    _enum_registry: &EnumRegistry,
) -> TypedExpr {
    let scrutinee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Sum(enum_name.to_string()),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: "__self".to_string(),
        },
    };
    let scrutinee = Spanned::new(scrutinee, Span::synthetic());

    let arms: Vec<TypedMatchArm> = variants
        .iter()
        .enumerate()
        .map(|(tag, variant)| build_enum_show_arm(enum_name, variant, tag))
        .collect();

    TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: true,
        escape: EscapeTarget::Ancestor(0),
        effect: Effect::Puro,
        kind: TypedExprKind::Match {
            scrutinee: Box::new(scrutinee),
            arms,
        },
    }
}

/// Constrói um braço de match para uma variante de enum.
fn build_enum_show_arm(
    enum_name: &str,
    variant: &kata_core::enum_registry::VariantInfo,
    tag: usize,
) -> TypedMatchArm {
    let (pattern, body) = match &variant.payload_ty {
        None => {
            // Variante unitária — pattern sem sub_patterns, body é TextLit.
            let pat = TypedPattern::Variant {
                enum_name: enum_name.to_string(),
                variant: variant.name.clone(),
                sub_patterns: None,
                tag,
            };
            let body = text_lit(variant.name.clone());
            (pat, body)
        }
        Some(payload_ty) => {
            // Variante com payload — sub_pattern Ident("v"), body é
            // string_concat("VariantName(", show v, ")").
            let sub_pat = Spanned::new(
                TypedPattern::Ident {
                    name: "v".to_string(),
                    ty: payload_ty.clone(),
                },
                Span::synthetic(),
            );
            let pat = TypedPattern::Variant {
                enum_name: enum_name.to_string(),
                variant: variant.name.clone(),
                sub_patterns: Some(vec![sub_pat]),
                tag,
            };

            // "VariantName(" ++ show v ++ ")"
            let prefix = text_lit(format!("{}(", variant.name));
            let v_expr = TypedExpr {
                span: Span::synthetic(),
                ty: payload_ty.clone(),
                tail_pos: false,
                escape: EscapeTarget::Local,
                effect: Effect::Puro,
                kind: TypedExprKind::Ident {
                    name: "v".to_string(),
                },
            };
            let v_spanned = Spanned::new(v_expr, Span::synthetic());

            let show_v = show_expr(v_spanned, payload_ty);
            let suffix = text_lit(")".to_string());

            let body_expr = string_concat(string_concat(prefix, show_v), suffix);

            (pat, body_expr)
        }
    };

    TypedMatchArm {
        pattern: Some(Spanned::new(pattern, Span::synthetic())),
        guard: None,
        body,
    }
}

// ════════════════════════════════════════════════════════════════
// List — síntese de show genérico recursivo
// ════════════════════════════════════════════════════════════════

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
    });

    // ── Registra `List implements SHOW` no InterfaceRegistry ──
    interface_registry
        .register_impl(ImplEntry {
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
        log: None,
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
        effect: Effect::Puro,
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
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: "h".to_string(),
        },
    };
    let h_spanned = Spanned::new(h_expr, Span::synthetic());

    // `show h` — despacha via show_expr (produz Closure com ffi_symbol: None
    // porque elem_ty é Ty::Var("A"); o monomorphizador resolve via Layer 5)
    let show_h = show_expr(h_spanned, elem_ty);

    // `t` como expr
    let t_expr = TypedExpr {
        span: Span::synthetic(),
        ty: list_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
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
        escape: EscapeTarget::Ancestor(0),
        effect: Effect::Puro,
        kind: TypedExprKind::Match {
            scrutinee: Box::new(scrutinee),
            arms: vec![cons_arm, nil_arm],
        },
    }
}
