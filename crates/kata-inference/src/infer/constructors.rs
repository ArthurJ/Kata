//! Síntese de smart constructors para structs e aliases.
//!
//! `data Pessoa (nome::Text idade::Int)` → overload `Pessoa :: Text Int => Pessoa`
//! no DispatchTable + TypedFunction com body `StructConstruct`.
//! `alias Float as Altura` → `Altura :: Float => Altura` (identity).

use kata_ast::Spanned;
use kata_core::dispatch::{DispatchTable, OverloadInfo};
use kata_core::escape::EscapeTarget;
use kata_core::struct_registry::StructRegistry;
use kata_core::ty::{Ty, TypeEnv};

use crate::typed::{TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause, TypedPattern};

/// Sintetiza smart constructors para structs com campos e aliases.
/// Registra overloads no `dispatch_table` e retorna as TypedFunctions sintetizadas.
pub(crate) fn synthesize_constructors(
    struct_registry: &StructRegistry,
    type_env: &TypeEnv,
    dispatch_table: &mut DispatchTable,
) -> Vec<TypedFunction> {
    let mut constructors = Vec::new();

    // 1a. Smart constructors para structs com campos (não-alias).
    for struct_name in struct_registry.names() {
        let struct_info = struct_registry
            .get(struct_name)
            .expect("struct_name veio de struct_registry.names()");
        // Aliases são processados no passo 1b abaixo — pular aqui.
        if struct_info.alias_of.is_some() {
            continue;
        }
        if struct_info.fields.is_empty() {
            continue; // struct sem campos = tipo opaco, não ganha construtor
        }

        let field_types: Vec<Ty> = struct_info.fields.iter().map(|f| f.ty.clone()).collect();
        let ret_ty = Ty::Struct(struct_name.to_string());

        // Registra overload no DispatchTable.
        dispatch_table.insert(OverloadInfo {
            name: struct_name.to_string(),
            params: field_types.clone(),
            ret: ret_ty.clone(),
            ffi_symbol: None, // função Kata pura
            is_action: false,
            is_generic: false,
            is_constructor: true,
            associative_neutral: None,
            type_params: vec![],
            substitutions: None,
            param_names: vec![],
        });

        // Sintetiza a TypedFunction com uma cláusula:
        // patterns = [__field_0, __field_1, ...]
        // body = StructConstruct { struct_name, values: [Ident(__field_0), ...] }
        let patterns: Vec<Spanned<TypedPattern>> = field_types
            .iter()
            .enumerate()
            .map(|(i, ty)| {
                Spanned::new(
                    TypedPattern::Ident {
                        name: format!("__field_{i}"),
                        ty: ty.clone(),
                    },
                    kata_ast::Span::synthetic(),
                )
            })
            .collect();

        let values: Vec<Spanned<TypedExpr>> = field_types
            .iter()
            .enumerate()
            .map(|(i, ty)| {
                Spanned::new(
                    TypedExpr {
                        span: kata_ast::Span::synthetic(),
                        ty: ty.clone(),
                        tail_pos: false,
                        escape: EscapeTarget::Local,
                        effect: crate::typed::Effect::Puro,
                        kind: TypedExprKind::Ident {
                            name: format!("__field_{i}"),
                        },
                    },
                    kata_ast::Span::synthetic(),
                )
            })
            .collect();

        let body = TypedExpr {
            span: kata_ast::Span::synthetic(),
            ty: ret_ty.clone(),
            tail_pos: true,
            // Smart constructor é função pura — todos os valores vão para
            // a caller_arena, igual ao escape derivado em
            // infer_expr_hinted quando ctx.ret_ty = None.
            escape: EscapeTarget::Caller,
            effect: crate::typed::Effect::Puro,
            kind: TypedExprKind::StructConstruct {
                struct_name: struct_name.to_string(),
                values,
            },
        };

        constructors.push(TypedFunction {
            name: struct_name.to_string(),
            param_types: field_types,
            ret_ty,
            clauses: vec![TypedLambdaClause {
                patterns,
                body: Spanned::new(body, kata_ast::Span::synthetic()),
                guards: Vec::new(),
                with_bindings: Vec::new(),
            }],
            log: None,
        });
    }

    // 1b. Smart constructors para aliases (newtypes).
    //     `alias Float as Altura` → `Altura :: Float => Altura` (identity).
    //     `alias Pessoa as Pessoa2` → `Pessoa2 :: Text Int => Pessoa2` (StructConstruct).
    //     Refined types (com predicates) são pulados aqui — o smart
    //     constructor falível é sintetizado em constructors_refined.
    for struct_name in struct_registry.names() {
        let struct_info = struct_registry
            .get(struct_name)
            .expect("struct_name veio de struct_registry.names()");
        let Some(ref target) = struct_info.alias_of else {
            continue; // não é alias
        };
        // Pula refined types — têm predicates, ganham construtor falível.
        if struct_info.predicates.is_some() {
            continue;
        }

        let ret_ty = Ty::Struct(struct_name.to_string());

        if struct_info.fields.is_empty() {
            // Alias de primitivo/opaco — construtor identity.
            // `Altura :: Float => Altura` — body é Ident(__field_0).
            let target_ty = type_env
                .lookup(target)
                .unwrap_or_else(|| panic!("alias target {target} não encontrado no TypeEnv"))
                .clone();

            dispatch_table.insert(OverloadInfo {
                name: struct_name.to_string(),
                params: vec![target_ty.clone()],
                ret: ret_ty.clone(),
                ffi_symbol: None,
                is_action: false,
                is_generic: false,
                is_constructor: true,
                associative_neutral: None,
                type_params: vec![],
                substitutions: None,
                param_names: vec![],
            });

            let pattern = Spanned::new(
                TypedPattern::Ident {
                    name: "__field_0".into(),
                    ty: target_ty.clone(),
                },
                kata_ast::Span::synthetic(),
            );
            let body = TypedExpr {
                span: kata_ast::Span::synthetic(),
                ty: ret_ty,
                tail_pos: true,
                escape: EscapeTarget::Caller,
                effect: crate::typed::Effect::Puro,
                kind: TypedExprKind::Ident {
                    name: "__field_0".into(),
                },
            };
            constructors.push(TypedFunction {
                name: struct_name.to_string(),
                param_types: vec![target_ty],
                ret_ty: Ty::Struct(struct_name.to_string()),
                clauses: vec![TypedLambdaClause {
                    patterns: vec![pattern],
                    body: Spanned::new(body, kata_ast::Span::synthetic()),
                    guards: Vec::new(),
                    with_bindings: Vec::new(),
                }],
                log: None,
            });
        } else {
            // Alias de struct com campos — mesmo construtor do struct nativo,
            // mas com struct_name = new_name.
            let field_types: Vec<Ty> = struct_info.fields.iter().map(|f| f.ty.clone()).collect();

            dispatch_table.insert(OverloadInfo {
                name: struct_name.to_string(),
                params: field_types.clone(),
                ret: ret_ty.clone(),
                ffi_symbol: None,
                is_action: false,
                is_generic: false,
                is_constructor: true,
                associative_neutral: None,
                type_params: vec![],
                substitutions: None,
                param_names: vec![],
            });

            let patterns: Vec<Spanned<TypedPattern>> = field_types
                .iter()
                .enumerate()
                .map(|(i, ty)| {
                    Spanned::new(
                        TypedPattern::Ident {
                            name: format!("__field_{i}"),
                            ty: ty.clone(),
                        },
                        kata_ast::Span::synthetic(),
                    )
                })
                .collect();

            let values: Vec<Spanned<TypedExpr>> = field_types
                .iter()
                .enumerate()
                .map(|(i, ty)| {
                    Spanned::new(
                        TypedExpr {
                            span: kata_ast::Span::synthetic(),
                            ty: ty.clone(),
                            tail_pos: false,
                            escape: EscapeTarget::Local,
                            effect: crate::typed::Effect::Puro,
                            kind: TypedExprKind::Ident {
                                name: format!("__field_{i}"),
                            },
                        },
                        kata_ast::Span::synthetic(),
                    )
                })
                .collect();

            let body = TypedExpr {
                span: kata_ast::Span::synthetic(),
                ty: ret_ty.clone(),
                tail_pos: true,
                escape: EscapeTarget::Caller,
                effect: crate::typed::Effect::Puro,
                kind: TypedExprKind::StructConstruct {
                    struct_name: struct_name.to_string(),
                    values,
                },
            };
            constructors.push(TypedFunction {
                name: struct_name.to_string(),
                param_types: field_types,
                ret_ty,
                clauses: vec![TypedLambdaClause {
                    patterns,
                    body: Spanned::new(body, kata_ast::Span::synthetic()),
                    guards: Vec::new(),
                    with_bindings: Vec::new(),
                }],
                log: None,
            });
        }
    }

    constructors
}
