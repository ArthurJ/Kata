//! Pass 2 — type-check dos corpos, inferência, dispatch por dominância.
//!
//! Consome `ResolvedModule` (TypeEnv + assinaturas + EnumRegistry) + `Module` (AST) e
//! produz `TypedModule` (TAST com `ty`, `tail_pos`, `effect` em cada nó).
//!
//! Algoritmo: `infer_module` popula o DispatchTable a partir das
//! `signatures`, depois `infer_expr` percorre a AST recursivamente,
//! despachando `Apply` via `DispatchTable::resolve` ou `call_indirect`
//! via `TypeEnv` lookup.

mod _match;
mod apply;
mod apply_lambda;
mod captures;
mod expr;
mod format_synthesis;
mod helpers;
mod lambda;
mod partial_dispatch;
mod recursion;
mod repr_synthesis;
mod sugar;
mod variant;
mod variant_qual;

use kata_ast::{Item, Module, Spanned};
use kata_core::dispatch::OverloadInfo;
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use kata_resolution::ResolvedModule;

use crate::desugar;
use crate::typed::{
    TypedAction, TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause, TypedModule,
    TypedPattern,
};

use self::apply_lambda::infer_lambda_body;
use self::expr::fits_return;
use self::expr::{InferCtx, infer_expr};
use self::helpers::{
    check_patterns, item_span_or_synthetic, populate_dispatch_table, process_with_bindings,
};

pub use self::helpers::InferResult;

/// Infere o tipo de um módulo completo.
///
/// Pipeline: popula DispatchTable → processa funções nomeadas → infere entry point.
/// Retorna `TypedModule` ou o primeiro erro de typeck encontrado.
pub fn infer_module(module: &Module, resolved: &ResolvedModule) -> InferResult<TypedModule> {
    // 1. Popula DispatchTable com as assinaturas (prelude + módulo)
    let mut dispatch_table = populate_dispatch_table(&resolved.signatures);

    // 1a. Registra Actions definidas pelo usuário no DispatchTable (is_action = true).
    //     Actions não têm ffi_symbol (são compiladas como funções Kata).
    for action_def in &resolved.actions {
        dispatch_table.insert(OverloadInfo {
            name: action_def.name.clone(),
            params: action_def.param_types.clone(),
            ret: action_def.return_type.clone(),
            ffi_symbol: None,
            is_action: true,
            is_generic: false,
            is_constructor: false,
            associative_neutral: None,
        });
    }

    // 1b. Fio 5 — sintetiza smart constructors para structs com campos.
    //     `data Pessoa (nome::Text idade::Int)` → overload `Pessoa :: Text Int => Pessoa`
    //     no DispatchTable + TypedFunction com body `StructConstruct`.
    let mut struct_constructors: Vec<TypedFunction> = Vec::new();
    for struct_name in resolved.struct_registry.names() {
        let struct_info = resolved.struct_registry.get(struct_name).unwrap();
        // Aliases são processados no passo 1c abaixo — pular aqui.
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
            // a arena raiz (Ancestor(0)), igual ao escape derivado em
            // infer_expr_hinted quando ctx.ret_ty = None.
            escape: EscapeTarget::Ancestor(0),
            effect: crate::typed::Effect::Puro,
            kind: TypedExprKind::StructConstruct {
                struct_name: struct_name.to_string(),
                values,
            },
        };

        struct_constructors.push(TypedFunction {
            name: struct_name.to_string(),
            param_types: field_types,
            ret_ty,
            clauses: vec![TypedLambdaClause {
                patterns,
                body: Spanned::new(body, kata_ast::Span::synthetic()),
                guards: Vec::new(),
                with_bindings: Vec::new(),
            }],
        });
    }

    // 1c. Fio 5 Fase 5 — sintetiza smart constructors para aliases (newtypes).
    //     `alias Float as Altura` → `Altura :: Float => Altura` (identity).
    //     `alias Pessoa as Pessoa2` → `Pessoa2 :: Text Int => Pessoa2` (StructConstruct).
    for struct_name in resolved.struct_registry.names() {
        let struct_info = resolved.struct_registry.get(struct_name).unwrap();
        let Some(ref target) = struct_info.alias_of else {
            continue; // não é alias
        };

        let ret_ty = Ty::Struct(struct_name.to_string());

        if struct_info.fields.is_empty() {
            // Alias de primitivo/opaco — construtor identity.
            // `Altura :: Float => Altura` — body é Ident(__field_0).
            let target_ty = resolved
                .type_env
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
                escape: EscapeTarget::Ancestor(0),
                effect: crate::typed::Effect::Puro,
                kind: TypedExprKind::Ident {
                    name: "__field_0".into(),
                },
            };
            struct_constructors.push(TypedFunction {
                name: struct_name.to_string(),
                param_types: vec![target_ty],
                ret_ty: Ty::Struct(struct_name.to_string()),
                clauses: vec![TypedLambdaClause {
                    patterns: vec![pattern],
                    body: Spanned::new(body, kata_ast::Span::synthetic()),
                    guards: Vec::new(),
                    with_bindings: Vec::new(),
                }],
            });
        } else {
            // Alias de struct com campos — mesmo construtor do struct nativo,
            // mas com struct_name = new_name.
            let field_types: Vec<Ty> =
                struct_info.fields.iter().map(|f| f.ty.clone()).collect();

            dispatch_table.insert(OverloadInfo {
                name: struct_name.to_string(),
                params: field_types.clone(),
                ret: ret_ty.clone(),
                ffi_symbol: None,
                is_action: false,
                is_generic: false,
                is_constructor: true,
                associative_neutral: None,
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
                escape: EscapeTarget::Ancestor(0),
                effect: crate::typed::Effect::Puro,
                kind: TypedExprKind::StructConstruct {
                    struct_name: struct_name.to_string(),
                    values,
                },
            };
            struct_constructors.push(TypedFunction {
                name: struct_name.to_string(),
                param_types: field_types,
                ret_ty,
                clauses: vec![TypedLambdaClause {
                    patterns,
                    body: Spanned::new(body, kata_ast::Span::synthetic()),
                    guards: Vec::new(),
                    with_bindings: Vec::new(),
                }],
            });
        }
    }

    // 1d. Fio 5 Fase 6 — sintetiza `repr` para structs com campos.
    //     `repr :: Pessoa => Text` no DispatchTable + TypedFunction com body
    //     que constrói "Pessoa(field0, field1, ...)" via string_concat FFI.
    let mut repr_functions = repr_synthesis::synthesize_repr_functions(
        &resolved.struct_registry,
        &mut dispatch_table,
    );

    // 2. Clona o TypeEnv do ResolvedModule — o typeck pode adicionar bindings
    //    locais (let) sem mutar o original.
    let mut type_env = resolved.type_env.clone();

    // 3. Processa funções nomeadas com corpo Kata (Fase 10).
    //    Cada função é inferida com os tipos da assinatura (não InferVar).
    //    A função também é registrada no TypeEnv como Ty::Function para permitir
    //    `let g := fat` (função como valor).
    let mut typed_functions: Vec<TypedFunction> = Vec::new();
    for func_def in &resolved.functions {
        let ctx = InferCtx {
            table: &dispatch_table,
            enum_registry: &resolved.enum_registry,
            struct_registry: &resolved.struct_registry,
            ret_ty: None,
            in_loop: false,
        };
        let typed_func = infer_named_function(func_def, &ctx, &resolved.type_env)?;
        // Registra no TypeEnv para permitir uso como valor (call_indirect).
        type_env.define(
            &typed_func.name,
            Ty::Function(
                typed_func.param_types.clone(),
                Box::new(typed_func.ret_ty.clone()),
            ),
        );
        typed_functions.push(typed_func);
    }

    // 3a. Processa Actions (Fio 3). Cada Action é inferida com os tipos
    //     da assinatura. O body é uma sequência de statements.
    let mut typed_actions: Vec<TypedAction> = Vec::new();
    for action_def in &resolved.actions {
        let ctx = InferCtx {
            table: &dispatch_table,
            enum_registry: &resolved.enum_registry,
            struct_registry: &resolved.struct_registry,
            ret_ty: Some(&action_def.return_type),
            in_loop: false,
        };
        let typed_action = infer_action(action_def, &ctx, &resolved.type_env)?;
        typed_actions.push(typed_action);
    }

    // 3b. Fase 11 — verifica que nenhuma Action é recursiva.
    //     Actions executam em fibers com stack fixa; recursão estouraria.
    recursion::check_action_recursion(&typed_actions)?;

    // 4. Percorre items — infere cada EntryExpr em sequência.
    //    O último vira o entry point; os anteriores viram pre_entry
    //    (lowerados em sequência pelo codegen, compartilhando var_map).
    let mut pre_entry: Vec<Spanned<TypedExpr>> = Vec::new();
    let mut entry_expr: Option<Spanned<TypedExpr>> = None;

    for item in &module.items {
        match &item.node {
            Item::EntryExpr(expr) => {
                // Desugar Pipe e Hole antes do typeck. Após isto, a AST
                // não contém Expr::Pipe nem Expr::Hole — o typeck nunca os
                // vê. Isto é total: TAST nunca contém Pipe nem Hole.
                let desugared = desugar::desugar(expr);
                let ctx = InferCtx {
                    table: &dispatch_table,
                    enum_registry: &resolved.enum_registry,
                    struct_registry: &resolved.struct_registry,
                    ret_ty: None,
                    in_loop: false,
                };
                let typed = infer_expr(
                    &desugared.node,
                    &desugared.span,
                    &mut type_env,
                    &ctx,
                    true, // entry point está em tail position
                )?;
                // Se já temos um entry_expr, ele vira pre_entry; o novo vira entry.
                if let Some(prev) = entry_expr.take() {
                    pre_entry.push(prev);
                }
                entry_expr = Some(Spanned::new(typed, expr.span));
            }
            Item::Sig { .. } | Item::DataDecl { .. } | Item::EnumDecl { .. }
            | Item::AliasDecl { .. } => {
                // Já processado no resolution/inference de funções nomeadas.
            }
            Item::ActionDecl { .. } => {
                // Já processado no inference de Actions (abaixo).
            }
        }
    }

    let entry = entry_expr.ok_or_else(|| MiddleError::UnboundName {
        name: "<entry point>".into(),
        span: item_span_or_synthetic(&module.items),
    })?;

    let mut typed_module = TypedModule {
        pre_entry,
        entry,
        dispatch_table,
        type_env,
        functions: {
            let mut all_funcs = typed_functions;
            all_funcs.extend(struct_constructors);
            all_funcs.append(&mut repr_functions);
            all_funcs
        },
        actions: typed_actions,
    };

    // Fase 12: coleta captures (free variables) de cada Closure.
    // Percorre a TAST já construída e muta in-place os campos `captures`.
    captures::run(&mut typed_module);

    Ok(typed_module)
}

/// Infere uma função nomeada com corpo Kata (múltiplas cláusulas).
///
/// Cada cláusula é inferida com os tipos da assinatura (param_types/ret_ty
/// do Sig). Os padrões são casados contra os tipos dos parâmetros. O corpo
/// de cada cláusula é inferido em escopo filho com os bindings dos padrões.
fn infer_named_function(
    func_def: &kata_resolution::FunctionDef,
    ctx: &InferCtx,
    module_type_env: &TypeEnv,
) -> InferResult<TypedFunction> {
    let param_types = &func_def.param_types;
    let ret_ty = &func_def.return_type;

    let mut typed_clauses: Vec<TypedLambdaClause> = Vec::new();

    for clause in &func_def.clauses {
        let clause_inner = &clause.node;

        // Cria escopo filho para a cláusula, com acesso aos tipos do módulo
        // (prelude + user). Sem parent, VariantQual como `Boolean::True` falha
        // com UnboundName — o typeck precisa resolver o nome do enum no TypeEnv.
        let mut clause_env = TypeEnv::with_parent(module_type_env.clone());

        // Casa padrões contra tipos dos parâmetros.
        let typed_patterns = check_patterns(
            &clause_inner.patterns,
            param_types,
            ctx.enum_registry,
            &mut clause_env,
        )?;

        // Processa with bindings (açúcar → let chain).
        let typed_with_bindings =
            process_with_bindings(&clause_inner.with_bindings, &mut clause_env, ctx)?;

        // Infere body (com ou sem guards).
        let (typed_body, typed_guards) = if clause_inner.guards.is_empty() {
            let typed_body = infer_expr(
                &clause_inner.body.node,
                &clause_inner.body.span,
                &mut clause_env,
                ctx,
                true, // tail_pos = true em body de função
            )?;
            // Verifica que o body retorna o tipo esperado.
            if typed_body.ty != *ret_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{:?}", ret_ty),
                    found: format!("{:?}", typed_body.ty),
                    span: clause_inner.body.span.into(),
                });
            }
            (typed_body, Vec::new())
        } else {
            let (guard_ret, typed_body, guards) = infer_lambda_body(
                &clause_inner.body,
                &clause_inner.guards,
                &mut clause_env,
                ctx,
            )?;
            if guard_ret != *ret_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{:?}", ret_ty),
                    found: format!("{:?}", guard_ret),
                    span: clause_inner.body.span.into(),
                });
            }
            (typed_body, guards)
        };

        typed_clauses.push(TypedLambdaClause {
            patterns: typed_patterns,
            body: Spanned::new(typed_body, clause_inner.body.span),
            guards: typed_guards,
            with_bindings: typed_with_bindings,
        });
    }

    // DoD 12: Verifica sobreposição de cláusulas (RedundantClause).
    // Uma cláusula é redundante se uma cláusula anterior já cobre todos
    // os valores que ela casaria, e a cláusula posterior não tem guards
    // (sem condição adicional que a diferenciaria).
    crate::redundancy::check_redundant_clauses(&func_def.clauses)?;

    Ok(TypedFunction {
        name: func_def.name.clone(),
        param_types: param_types.clone(),
        ret_ty: ret_ty.clone(),
        clauses: typed_clauses,
    })
}

/// Infere uma Action — produz `TypedAction` a partir de `ActionDef`.
///
/// O body é uma sequência de statements (`ActionStmt`). Cada statement é
/// inferido em sequência no mesmo escopo. O último statement **sem `;`** é
/// o retorno implícito — verifica tipo contra `ret_ty`. O último statement
/// **com `;`** retorna `Unit` — se `ret_ty` não for `Unit`, é um erro.
/// Após um `return`, statements subsequentes são unreachable — paramos.
fn infer_action(
    action_def: &kata_resolution::ActionDef,
    ctx: &InferCtx,
    module_type_env: &TypeEnv,
) -> InferResult<TypedAction> {
    let param_types = &action_def.param_types;
    let ret_ty = &action_def.return_type;

    // Cria escopo para a Action com o type_env do módulo como parent.
    // Isso permite que o body da Action acesse tipos do prelude (Result, Optional, etc).
    let mut action_env = TypeEnv::with_parent(module_type_env.clone());

    // Define parâmetros no escopo.
    for (i, ty) in param_types.iter().enumerate() {
        action_env.define(&format!("__param_{i}"), ty.clone());
    }

    // Infere cada statement do body em sequência.
    // O último statement sem `;` é o retorno implícito (tail_pos = true).
    // O último statement com `;` retorna Unit (tail_pos = false).
    // Após um `return`, statements subsequentes são unreachable — paramos.
    let mut typed_body: Vec<Spanned<TypedExpr>> = Vec::new();
    let n = action_def.body.len();
    for (i, stmt) in action_def.body.iter().enumerate() {
        let is_last = i == n - 1;
        // Se o último statement tem `;`, não é retorno implícito.
        let tail_pos = is_last && !stmt.has_semicolon;
        let desugared = desugar::desugar(&stmt.expr);
        let typed = infer_expr(
            &desugared.node,
            &desugared.span,
            &mut action_env,
            ctx,
            tail_pos,
        )?;
        let is_return = matches!(typed.kind, TypedExprKind::Return(_));
        typed_body.push(Spanned::new(typed, stmt.expr.span));
        if is_return {
            break; // statements após return são unreachable
        }
    }

    // Verifica que o último statement produz o tipo esperado.
    // Se o body terminou com `return`, o tipo já foi validado em infer_return.
    // Senão, o último statement é o retorno implícito (ou Unit se tinha `;`).
    if let Some(last) = typed_body.last()
        && !matches!(last.node.kind, TypedExprKind::Return(_))
    {
        let actual_ty = &last.node.ty;
        // Se o último statement tinha `;`, o retorno é Unit.
        let expected_ty = if action_def.body.last().is_some_and(|s| s.has_semicolon) {
            &Ty::Unit
        } else {
            ret_ty
        };
        if !fits_return(actual_ty, expected_ty) {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{expected_ty:?}"),
                found: format!("{actual_ty:?}"),
                span: last.span.into(),
            });
        }
    }

    Ok(TypedAction {
        name: action_def.name.clone(),
        param_types: param_types.clone(),
        ret_ty: ret_ty.clone(),
        body: typed_body,
    })
}
