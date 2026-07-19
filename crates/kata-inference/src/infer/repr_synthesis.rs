//! Síntese de `repr` — função auto-sintetizada para `data` com campos.
//!
//! Para cada `data Nome (campos)`, o typeck sintetiza `repr :: Nome => Text`
//! no DispatchTable + `TypedFunction` com body que constrói a string
//! `Nome(field0, field1, ...)` via `kata_rt_string_concat` (FFI binário).
//!
//! Por tipo de campo:
//! - `Text`: identity (o valor já é Text)
//! - `Int`: `kata_rt_int_to_text` via FFI
//! - `Boolean`: `kata_rt_bool_to_text` via FFI
//! - `Float`: `kata_rt_float_to_text` via FFI (quando existir)
//! - `Struct`: recursivo — chama `repr` do struct aninhado
//! - Outros: fallback não suportado neste fio

use kata_ast::{Span, Spanned};
use kata_core::dispatch::{DispatchTable, OverloadInfo};
use kata_core::escape::EscapeTarget;
use kata_core::struct_registry::StructRegistry;
use kata_core::ty::{PrimTy, Ty};

use crate::typed::{
    Effect, TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause, TypedPattern,
};

/// Síntese de funções `repr` para todos os structs com campos.
///
/// Retorna `Vec<TypedFunction>` que deve ser adicionada a `typed_module.functions`
/// e os overloads já registrados no `dispatch_table`.
pub(crate) fn synthesize_repr_functions(
    struct_registry: &StructRegistry,
    dispatch_table: &mut DispatchTable,
) -> Vec<TypedFunction> {
    let mut repr_functions = Vec::new();

    for struct_name in struct_registry.names() {
        let struct_info = struct_registry
            .get(struct_name)
            .expect("struct_name veio de struct_registry.names()");

        // Aliases não ganham repr próprio — usam o repr do target.
        if struct_info.alias_of.is_some() {
            continue;
        }

        // Structs sem campos não ganham repr.
        if struct_info.fields.is_empty() {
            continue;
        }

        let ret_ty = Ty::text();
        let param_ty = Ty::Struct(struct_name.to_string());

        // Registra overload `repr :: Struct => Text` no DispatchTable.
        // O nome no DispatchTable é "repr" (despacho por tipo). O nome da
        // TypedFunction é mangled (`repr__Pessoa`) para evitar duplicate
        // definition no Cranelift quando há múltiplos structs.
        dispatch_table.insert(OverloadInfo {
            name: "repr".to_string(),
            params: vec![param_ty.clone()],
            ret: ret_ty.clone(),
            ffi_symbol: Some(format!("__kata_repr__{struct_name}")),
            is_action: false,
            is_generic: false,
            is_constructor: false,
            associative_neutral: None,
            type_params: vec![],
            substitutions: None,
        });

        // Pattern: `__self` : Struct
        let pattern = Spanned::new(
            TypedPattern::Ident {
                name: "__self".to_string(),
                ty: param_ty.clone(),
            },
            Span::synthetic(),
        );

        // Constrói o body: string_concat aninhado
        // "Nome(" ++ field0_repr ++ ", " ++ field1_repr ++ ... ++ ")"
        let body = build_repr_body(struct_name, &struct_info.fields, struct_registry);

        repr_functions.push(TypedFunction {
            name: format!("__kata_repr__{struct_name}"),
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

    repr_functions
}

/// Constrói o body de `repr` como uma árvore aninhada de `string_concat`.
///
/// `Nome(f0, f1, ..., fN)` =
///   concat("Nome(", concat(f0_repr, concat(", ", concat(f1_repr, ... concat(")"))))
fn build_repr_body(
    struct_name: &str,
    fields: &[kata_core::struct_registry::FieldInfo],
    struct_registry: &StructRegistry,
) -> TypedExpr {
    // Constrói a lista de strings a concatenar:
    // "Nome(" , field0_repr, ", ", field1_repr, ", ", ..., ")"
    let mut parts: Vec<Spanned<TypedExpr>> = Vec::new();

    // Prefixo "Nome("
    parts.push(text_lit(format!("{struct_name}(")));

    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            parts.push(text_lit(", ".to_string()));
        }
        parts.push(field_repr(field, i, struct_registry));
    }

    // Sufixo ")"
    parts.push(text_lit(")".to_string()));

    // Reduz a lista com string_concat (left-associative):
    // concat(parts[0], concat(parts[1], concat(parts[2], ...)))
    let result = parts.into_iter().reduce(string_concat);

    let body = result.expect("repr body tem pelo menos 2 parts");

    TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: true,
        escape: EscapeTarget::Ancestor(0),
        effect: Effect::Puro,
        kind: body.node.kind,
    }
}

/// Produz a representação Text de um campo.
///
/// - `Text`: identity (FieldAccess direto)
/// - `Int`: `int_to_text(field_access)`
/// - `Boolean`: `bool_to_text(field_access)`
/// - `Float`: não suportado neste fio (sem FFI)
/// - `Struct`: chamada recursiva a `repr` (call direto)
fn field_repr(
    field: &kata_core::struct_registry::FieldInfo,
    field_index: usize,
    _struct_registry: &StructRegistry,
) -> Spanned<TypedExpr> {
    let field_access = field_access_expr(field_index, &field.ty);

    match &field.ty {
        Ty::Prim(PrimTy::Text) => {
            // Text → identity
            field_access
        }
        Ty::Prim(PrimTy::Int) => {
            // Int → int_to_text
            ffi_call1("kata_rt_int_to_text", field_access, Ty::text())
        }
        Ty::Prim(PrimTy::Rational) => {
            // Rational → rat_show
            ffi_call1("kata_rt_rat_show", field_access, Ty::text())
        }
        Ty::Prim(PrimTy::Float) => {
            // Float → kata_rt_float_to_text (f64 -> text ptr)
            ffi_call1("kata_rt_float_to_text", field_access, Ty::text())
        }
        Ty::Sum(name) if name == "Boolean" => {
            // Boolean → bool_to_text
            ffi_call1("kata_rt_bool_to_text", field_access, Ty::text())
        }
        Ty::Struct(name) => {
            // Struct aninhado → chamada recursiva a `repr`
            // repr é uma função Kata nomeada — call direto (sem ffi_symbol).
            repr_call(field_access, name.clone())
        }
        _ => {
            // Fallback: int_to_text (mostra como número)
            ffi_call1("kata_rt_int_to_text", field_access, Ty::text())
        }
    }
}

/// Constrói `FieldAccess { expr: __self, field_index }`.
fn field_access_expr(field_index: usize, field_ty: &Ty) -> Spanned<TypedExpr> {
    let self_expr = TypedExpr {
        span: Span::synthetic(),
        ty: field_ty.clone(), // será sobrescrito pelo outer
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
                struct_name: String::new(), // não usado pelo codegen
                field_name: String::new(),  // não usado pelo codegen
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

/// Constrói chamada recursiva a `repr` (call direto via ffi_symbol mangled).
/// `repr(field_access)` → chama `__kata_repr__{struct_name}`.
fn repr_call(field_access: Spanned<TypedExpr>, struct_name: String) -> Spanned<TypedExpr> {
    let mangled = format!("__kata_repr__{struct_name}");
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![Ty::Struct(struct_name.clone())], Box::new(Ty::text())),
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
                args: vec![field_access],
                ffi_symbol: Some(mangled),
            },
        },
        Span::synthetic(),
    )
}
