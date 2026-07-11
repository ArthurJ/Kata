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
mod expr;
mod helpers;
mod lambda;
mod partial_dispatch;

use kata_ast::{Item, Module, Spanned};
use kata_core::dispatch::{DispatchTable, OverloadInfo};
use kata_core::enum_registry::EnumRegistry;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use kata_resolution::ResolvedModule;

use crate::desugar;
use crate::typed::{TypedAction, TypedExpr, TypedFunction, TypedLambdaClause, TypedModule};

use self::apply_lambda::infer_lambda_body;
use self::expr::infer_expr;
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

    // 2. Clona o TypeEnv do ResolvedModule — o typeck pode adicionar bindings
    //    locais (let) sem mutar o original.
    let mut type_env = resolved.type_env.clone();

    // 3. Processa funções nomeadas com corpo Kata (Fase 10).
    //    Cada função é inferida com os tipos da assinatura (não InferVar).
    //    A função também é registrada no TypeEnv como Ty::Function para permitir
    //    `let g := fat` (função como valor).
    let mut typed_functions: Vec<TypedFunction> = Vec::new();
    for func_def in &resolved.functions {
        let typed_func = infer_named_function(func_def, &dispatch_table, &resolved.enum_registry)?;
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
        let typed_action = infer_action(action_def, &dispatch_table, &resolved.enum_registry)?;
        typed_actions.push(typed_action);
    }

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
                let typed = infer_expr(
                    &desugared.node,
                    &desugared.span,
                    &mut type_env,
                    &dispatch_table,
                    &resolved.enum_registry,
                    true, // entry point está em tail position
                )?;
                // Se já temos um entry_expr, ele vira pre_entry; o novo vira entry.
                if let Some(prev) = entry_expr.take() {
                    pre_entry.push(prev);
                }
                entry_expr = Some(Spanned::new(typed, expr.span));
            }
            Item::Sig { .. } | Item::DataDecl { .. } | Item::EnumDecl { .. } => {
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

    Ok(TypedModule {
        pre_entry,
        entry,
        dispatch_table,
        type_env,
        functions: typed_functions,
        actions: typed_actions,
    })
}

/// Infere uma função nomeada com corpo Kata (múltiplas cláusulas).
///
/// Cada cláusula é inferida com os tipos da assinatura (param_types/ret_ty
/// do Sig). Os padrões são casados contra os tipos dos parâmetros. O corpo
/// de cada cláusula é inferido em escopo filho com os bindings dos padrões.
fn infer_named_function(
    func_def: &kata_resolution::FunctionDef,
    table: &DispatchTable,
    enum_registry: &EnumRegistry,
) -> InferResult<TypedFunction> {
    let param_types = &func_def.param_types;
    let ret_ty = &func_def.return_type;

    let mut typed_clauses: Vec<TypedLambdaClause> = Vec::new();

    for clause in &func_def.clauses {
        let clause_inner = &clause.node;

        // Cria escopo filho para a cláusula.
        let mut clause_env = TypeEnv::new();

        // Casa padrões contra tipos dos parâmetros.
        let typed_patterns = check_patterns(
            &clause_inner.patterns,
            param_types,
            enum_registry,
            &mut clause_env,
        )?;

        // Processa with bindings (açúcar → let chain).
        let typed_with_bindings = process_with_bindings(
            &clause_inner.with_bindings,
            &mut clause_env,
            table,
            enum_registry,
        )?;

        // Infere body (com ou sem guards).
        let (typed_body, typed_guards) = if clause_inner.guards.is_empty() {
            let typed_body = infer_expr(
                &clause_inner.body.node,
                &clause_inner.body.span,
                &mut clause_env,
                table,
                enum_registry,
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
                table,
                enum_registry,
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
/// O body é uma sequência de statements. Cada statement é inferido em
/// sequência no mesmo escopo. O último statement (sem `;`) é o retorno
/// implícito — verifica tipo contra `ret_ty`.
fn infer_action(
    action_def: &kata_resolution::ActionDef,
    table: &DispatchTable,
    enum_registry: &EnumRegistry,
) -> InferResult<TypedAction> {
    let param_types = &action_def.param_types;
    let ret_ty = &action_def.return_type;

    // Cria escopo para a Action.
    let mut action_env = TypeEnv::new();

    // Define parâmetros no escopo.
    for (i, ty) in param_types.iter().enumerate() {
        action_env.define(&format!("__param_{i}"), ty.clone());
    }

    // Infere cada statement do body em sequência.
    // O último statement é o retorno implícito (tail_pos = true).
    let mut typed_body: Vec<Spanned<TypedExpr>> = Vec::new();
    let n = action_def.body.len();
    for (i, stmt) in action_def.body.iter().enumerate() {
        let is_last = i == n - 1;
        let desugared = desugar::desugar(stmt);
        let typed = infer_expr(
            &desugared.node,
            &desugared.span,
            &mut action_env,
            table,
            enum_registry,
            is_last, // último statement é retorno implícito (tail_pos)
        )?;
        typed_body.push(Spanned::new(typed, stmt.span));
    }

    // Verifica que o último statement retorna o tipo esperado.
    if let Some(last) = typed_body.last()
        && last.node.ty != *ret_ty
    {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{ret_ty:?}"),
            found: format!("{:?}", last.node.ty),
            span: last.span.into(),
        });
    }

    Ok(TypedAction {
        name: action_def.name.clone(),
        param_types: param_types.clone(),
        ret_ty: ret_ty.clone(),
        body: typed_body,
    })
}
