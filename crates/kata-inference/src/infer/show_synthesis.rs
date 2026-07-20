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
// Helpers compartilhados
// ════════════════════════════════════════════════════════════════

/// Produz uma expressão `show <expr>` que despacha para a implementação
/// correta de SHOW do tipo. Usado dentro de `show` sintetizado para enums
/// (para mostrar o payload da variante).
///
/// Para tipos com `show` sintetizado (Struct, Sum), chama o mangled.
/// Para primitivos (Int, Float, Rational, Text), chama a FFI direto.
/// Para `Ty::Var` (enum genérico) onde o type param foi resolvido para
/// um tipo concreto, despacha via iface method (Caminho 0).
/// Para `Ty::Var` onde o type param não foi resolvido (ex: `E` em
/// `Result::Ok 42` — o `Err` nunca é usado), produz o fallback `"?"`
/// — não há tipo concreto para despachar.
fn show_expr(arg: Spanned<TypedExpr>, arg_ty: &Ty) -> Spanned<TypedExpr> {
    match arg_ty {
        Ty::Prim(PrimTy::Int) => ffi_call1("kata_rt_bi_show", arg, Ty::text()),
        Ty::Prim(PrimTy::Float) => ffi_call1("kata_rt_float_to_text", arg, Ty::text()),
        Ty::Prim(PrimTy::Rational) => ffi_call1("kata_rt_rat_show", arg, Ty::text()),
        Ty::Prim(PrimTy::Text) => arg, // identity
        Ty::Sum(name) => show_call(arg, name.clone(), arg_ty),
        Ty::Struct(name) => show_call(arg, name.clone(), arg_ty),
        Ty::Var(name) => {
            // Ty::Var em enum genérico. Produz `show v` com ffi_symbol: None.
            // O monomorphizador, ao instanciar a função sintetizada, substitui
            // Var("T") → tipo concreto (ex: Int) via apply_subs. O Layer 5
            // (resolução de ffi_symbol post-instantiation) encontra o overload
            // concreto de `show` para o tipo resolvido e preenche ffi_symbol.
            // Se o type param não for resolvido (ex: `E` em `Result::Ok 42`),
            // o Layer 5 não encontra overload e o ffi_symbol fica None —
            // o codegen produz erro gracioso (não SIGSEGV).
            let _ = name;
            let callee_ty = Ty::Function(vec![arg_ty.clone()], Box::new(Ty::text()));
            let callee = TypedExpr {
                span: Span::synthetic(),
                ty: callee_ty,
                tail_pos: false,
                escape: EscapeTarget::Local,
                effect: Effect::Puro,
                kind: TypedExprKind::Ident {
                    name: "show".to_string(),
                },
            };
            Spanned::new(
                TypedExpr {
                    span: Span::synthetic(),
                    ty: Ty::text(),
                    tail_pos: false,
                    escape: EscapeTarget::Ancestor(0),
                    effect: Effect::Puro,
                    kind: TypedExprKind::Closure {
                        callee: Box::new(Spanned::new(callee, Span::synthetic())),
                        args: vec![arg],
                        ffi_symbol: None,
                    },
                },
                Span::synthetic(),
            )
        }
        _ => {
            // Fallback para tipos não cobertos — int_to_text como saída segura.
            ffi_call1("kata_rt_int_to_text", arg, Ty::text())
        }
    }
}

/// Constrói chamada a `__kata_show__{Type}` mangled (call direto via ffi_symbol).
fn show_call(arg: Spanned<TypedExpr>, type_name: String, arg_ty: &Ty) -> Spanned<TypedExpr> {
    let mangled = format!("__kata_show__{type_name}");
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![arg_ty.clone()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: mangled.clone(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Ancestor(0),
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![arg],
                ffi_symbol: Some(mangled),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `FieldAccess { expr: __self, field_index }`.
fn field_access_expr(field_index: usize, field_ty: &Ty) -> Spanned<TypedExpr> {
    let self_expr = TypedExpr {
        span: Span::synthetic(),
        ty: field_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: "__self".to_string(),
        },
    };
    let self_spanned = Spanned::new(self_expr, Span::synthetic());

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: field_ty.clone(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::FieldAccess {
                expr: Box::new(self_spanned),
                struct_name: String::new(),
                field_name: String::new(),
                field_index: field_index as u32,
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `TextLit(text)`.
fn text_lit(text: String) -> Spanned<TypedExpr> {
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::TextLit { text },
        },
        Span::synthetic(),
    )
}

/// Constrói `Closure { callee=Ident(ffi), args=[arg], ffi_symbol=Some(ffi) }`.
fn ffi_call1(ffi_name: &str, arg: Spanned<TypedExpr>, ret_ty: Ty) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![arg.node.ty.clone()], Box::new(ret_ty.clone())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: ffi_name.to_string(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: ret_ty,
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![arg],
                ffi_symbol: Some(ffi_name.to_string()),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `string_concat(left, right)` — FFI call binário.
fn string_concat(left: Spanned<TypedExpr>, right: Spanned<TypedExpr>) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![Ty::text(), Ty::text()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: "kata_rt_string_concat".to_string(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Ancestor(0),
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![left, right],
                ffi_symbol: Some("kata_rt_string_concat".to_string()),
            },
        },
        Span::synthetic(),
    )
}
