//! Pass 2 — type-check dos corpos, inferência, dispatch por dominância.
//!
//! Consome `ResolvedModule` (TypeEnv + assinaturas + EnumRegistry) + `Module` (AST) e
//! produz `TypedModule` (TAST com `ty`, `tail_pos` em cada nó).
//!
//! Algoritmo: `infer_module` popula o DispatchTable a partir das
//! `signatures`, depois `infer_expr` percorre a AST recursivamente,
//! despachando `Apply` via `DispatchTable::resolve` ou `call_indirect`
//! via `TypeEnv` lookup.

mod _match;
mod action_call;
mod action_infer;
mod apply;
mod apply_lambda;
mod ascription;
mod captures;
mod collections;
mod collections_hof;
mod const_eval;
mod constructors;
mod constructors_enum_pred;
mod constructors_refined;
mod csp;
mod csp_builtins;
mod cycle;
mod dict_set;
mod dot_access;
mod expr;
mod format_synthesis;
mod free_vars;
pub(crate) mod generics;
pub(crate) mod helpers;
mod iface_dispatch;
mod lambda;
mod log_builtins;
mod log_synthesis;
mod partial_dispatch;
mod recursion;
mod refined_builders;
mod show_synthesis;
mod show_synthesis_helpers;
mod show_synthesis_list;
mod sugar;
mod variant;
mod variant_construct;
mod variant_qual;
mod walk;

use action_infer::infer_action;
use kata_ast::{GuardClause, Item, Module, Spanned, WithBinding};
use kata_core::dispatch::OverloadInfo;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use kata_resolution::ResolvedModule;
use kata_resolution::collect_type_params;

use crate::desugar;
use crate::typed::{TypedAction, TypedExpr, TypedFunction, TypedLambdaClause, TypedModule};

use self::apply_lambda::infer_lambda_body;
use self::expr::{InferCtx, infer_expr, infer_expr_hinted};
use self::helpers::{
    InferResult, check_patterns, item_span_or_synthetic, populate_dispatch_table,
    process_with_bindings,
};

/// Infere o tipo de um módulo completo.
///
/// Pipeline: popula DispatchTable → processa funções nomeadas → infere entry point.
/// Retorna `TypedModule` ou o primeiro erro de typeck encontrado.
pub fn infer_module(
    module: &Module,
    resolved: &ResolvedModule,
) -> Result<TypedModule, MiddleError> {
    // 1. Popula DispatchTable com as assinaturas (prelude + módulo)
    let mut dispatch_table = populate_dispatch_table(&resolved.signatures);

    // Clone mutável do InterfaceRegistry — a síntese de `show` registra
    // impls de SHOW para structs e enums aqui, e o typeck (InferCtx)
    // precisa enxergá-los para despachar `show` corretamente.
    let mut interface_registry = resolved.interface_registry.clone();

    // 0. Validação post-merge de `refines`: verifica que o tipo base implementa
    //     a interface delegada. Esta validação só pode ser feita aqui porque o
    //     prelude (com `Int implements NUM`) é mergeado antes de infer_module.
    for refines_entry in resolved.refines_registry.all_entries() {
        let base_ty_name = ty_name(&refines_entry.base_ty);
        if !base_ty_name.is_empty()
            && !interface_registry.type_implements(base_ty_name, &refines_entry.interface_name)
        {
            return Err(MiddleError::NoOverload {
                name: format!(
                    "refines: tipo base {base_ty_name} não implementa a interface {}",
                    refines_entry.interface_name
                ),
                span: kata_ast::Span::synthetic().into(),
            });
        }
    }

    // 1a. Registra Actions definidas pelo usuário no DispatchTable (is_action = true).
    //     Actions não têm ffi_symbol (são compiladas como funções Kata).
    //     Coleta type params (Ty::Var UPPER_CASE e Ty::Interface) para habilitar
    //     monomorfização de Actions polimórficas por interface (ex: echo :: SHOW).
    for action_def in &resolved.actions {
        let type_params = collect_type_params(&action_def.param_types, &action_def.return_type);
        let is_generic = !type_params.is_empty();
        dispatch_table.insert(OverloadInfo {
            name: action_def.name.clone(),
            params: action_def.param_types.clone(),
            ret: action_def.return_type.clone(),
            ffi_symbol: None,
            is_action: true,
            is_generic,
            is_constructor: false,
            associative_neutral: None,
            type_params,
            substitutions: None,
            param_names: action_def.param_names.clone(),
        });
    }

    // 1a/1c. sintetiza smart constructors para structs com campos e aliases.
    let struct_constructors = constructors::synthesize_constructors(
        &resolved.struct_registry,
        &resolved.type_env,
        &mut dispatch_table,
    )?;

    // 1e. sintetiza funções predicado e smart constructors
    //     falíveis para tipos refinados (`data (Int, > _ 0) as PositiveInt`).
    let refined_constructors = constructors_refined::synthesize_refined(
        &resolved.refined_decls,
        &resolved.enum_registry,
        &resolved.struct_registry,
        &resolved.type_env,
        &mut dispatch_table,
    )?;

    // 1f. sintetiza construtores despachadores para enums com
    //     variantes predicadas (`enum IMC: Magreza(< _ 18.5), ...`).
    let enum_pred_constructors = constructors_enum_pred::synthesize_enum_pred(
        &resolved.enum_pred_decls,
        &resolved.enum_registry,
        &resolved.struct_registry,
        &resolved.type_env,
        &mut dispatch_table,
    )?;

    // 1d. sintetiza `show` para structs com campos e todos os enums.
    //     `show :: Pessoa => Text` no DispatchTable + TypedFunction com body
    //     que constrói "Pessoa(field0, field1, ...)" via string_concat FFI.
    //     `show :: Boolean => Text` etc. para enums (Match sobre variantes).
    //     Registra `Struct/Enum implements SHOW` no InterfaceRegistry.
    let mut show_functions = show_synthesis::synthesize_show_functions(
        &resolved.struct_registry,
        &resolved.enum_registry,
        &mut dispatch_table,
        &mut interface_registry,
    );

    // 1e. sintetiza `show` para List::A — duas funções genéricas mutuamente
    //     recursivas (__kata_show__List e __kata_show__List_rest) com pattern
    //     matching Cons/Nil. Registra `List implements SHOW` (type_params: ["A"]).
    let list_show_functions = show_synthesis_list::synthesize_list_show_functions(
        &mut dispatch_table,
        &mut interface_registry,
    );
    show_functions.extend(list_show_functions);

    // 2. Clona o TypeEnv do ResolvedModule — o typeck pode adicionar bindings
    //    locais (let) sem mutar o original.
    let mut type_env = resolved.type_env.clone();

    // 3. Processa funções nomeadas com corpo Kata.
    //    Cada função é inferida com os tipos da assinatura (não InferVar).
    //    A função também é registrada no TypeEnv como Ty::Function para permitir
    //    `let g := fat` (função como valor).
    let mut typed_functions: Vec<TypedFunction> = Vec::new();
    for func_def in &resolved.functions {
        let ctx = InferCtx {
            table: &dispatch_table,
            enum_registry: &resolved.enum_registry,
            struct_registry: &resolved.struct_registry,
            refined_decls: &resolved.refined_decls,
            interface_registry: &interface_registry,
            refines_registry: &resolved.refines_registry,
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
            "__local__",
        );
        typed_functions.push(typed_func);
    }

    // 3a. Processa Actions. Cada Action é inferida com os tipos
    //     da assinatura. O body é uma sequência de statements.
    let mut typed_actions: Vec<TypedAction> = Vec::new();
    for action_def in &resolved.actions {
        let ctx = InferCtx {
            table: &dispatch_table,
            enum_registry: &resolved.enum_registry,
            struct_registry: &resolved.struct_registry,
            refined_decls: &resolved.refined_decls,
            interface_registry: &interface_registry,
            refines_registry: &resolved.refines_registry,
            ret_ty: Some(&action_def.return_type),
            in_loop: false,
        };
        let typed_action = infer_action(action_def, &ctx, &resolved.type_env)?;
        typed_actions.push(typed_action);
    }

    // 3b. verifica que nenhuma Action é recursiva.
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
                    refined_decls: &resolved.refined_decls,
                    interface_registry: &interface_registry,
                    refines_registry: &resolved.refines_registry,
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
            Item::Sig { .. }
            | Item::DataDecl { .. }
            | Item::EnumDecl { .. }
            | Item::AliasDecl { .. }
            | Item::InterfaceDecl { .. }
            | Item::ImplementsDecl { .. }
            | Item::RefinesDecl { .. }
            | Item::ImportDecl { .. }
            | Item::ExportDecl { .. } => {
                // Já processado no resolution/inference de funções nomeadas.
                // Interfaces/implements/import/export são processados
                // (resolution) — o inference não os processa.
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
            all_funcs.extend(refined_constructors);
            all_funcs.extend(enum_pred_constructors);
            all_funcs.append(&mut show_functions);
            all_funcs
        },
        actions: typed_actions,
    };

    // Coleta captures (free variables) de cada Closure.
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

        // Desugar: elimina Pipe e Hole da cláusula antes do typeck.
        // O parser produz Expr::Hole para `_` em expressões como `< _ pivo`;
        // o desugar converte isso em Lambda. Sem isso, o typeck vê Hole cru
        // e falha com "Hole deve ter sido desugared".
        let desugared_body = crate::desugar::desugar(&clause_inner.body);
        let desugared_guards: Vec<GuardClause> = clause_inner
            .guards
            .iter()
            .map(|g| GuardClause {
                condition: g.condition.as_ref().map(crate::desugar::desugar),
                body: crate::desugar::desugar(&g.body),
            })
            .collect();
        let desugared_with_bindings: Vec<WithBinding> = clause_inner
            .with_bindings
            .iter()
            .map(|w| WithBinding {
                name: w.name.clone(),
                value: crate::desugar::desugar(&w.value),
            })
            .collect();

        // Casa padrões contra tipos dos parâmetros.
        let typed_patterns = check_patterns(
            &clause_inner.patterns,
            param_types,
            ctx.enum_registry,
            &mut clause_env,
        )?;

        // Processa with bindings (açúcar → let chain).
        let typed_with_bindings =
            process_with_bindings(&desugared_with_bindings, &mut clause_env, ctx)?;

        // Infere body (com ou sem guards).
        let (typed_body, typed_guards) = if desugared_guards.is_empty() {
            let typed_body = infer_expr_hinted(
                &desugared_body.node,
                &desugared_body.span,
                &mut clause_env,
                ctx,
                true,         // tail_pos = true em body de função
                Some(ret_ty), // hint = tipo de retorno da assinatura
            )?;
            // Verifica que o body retorna o tipo esperado.
            if typed_body.ty != *ret_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{}", ret_ty),
                    found: format!("{}", typed_body.ty),
                    span: clause_inner.body.span.into(),
                });
            }
            (typed_body, Vec::new())
        } else {
            let (guard_ret, typed_body, guards) =
                infer_lambda_body(&desugared_body, &desugared_guards, &mut clause_env, ctx)?;
            if guard_ret != *ret_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{}", ret_ty),
                    found: format!("{}", guard_ret),
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
    crate::redundancy::check_redundant_clauses(&typed_clauses)?;

    // Sintetiza log spec se a função tem @log.
    let log = if let Some(log_spec) = &func_def.log {
        // Extrai nomes dos params dos patterns da primeira cláusula.
        let param_names: Vec<String> = typed_clauses
            .first()
            .map(|c| {
                c.patterns
                    .iter()
                    .filter_map(|p| match &p.node {
                        crate::typed_pattern::TypedPattern::Ident { name, .. } => {
                            Some(name.clone())
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Cria escopo com bindings dos params para inferir o template.
        let mut log_env = TypeEnv::with_parent(module_type_env.clone());
        for (i, ty) in param_types.iter().enumerate() {
            log_env.define(&format!("__param_{i}"), ty.clone(), "__local__");
        }
        // Se há nomes de params, define-os também (associados por posição).
        for (i, name) in param_names.iter().enumerate() {
            if let Some(ty) = param_types.get(i) {
                log_env.define(name, ty.clone(), "__local__");
            }
        }
        Some(log_synthesis::synthesize_log_spec(
            log_spec,
            &param_names,
            &mut log_env,
            ctx,
        )?)
    } else {
        None
    };

    Ok(TypedFunction {
        name: func_def.name.clone(),
        param_types: param_types.clone(),
        ret_ty: ret_ty.clone(),
        clauses: typed_clauses,
        log,
    })
}

/// Extrai o nome de um `Ty` para validação de `refines`.
/// `Ty::Prim(PrimTy::Int)` → `"Int"`, `Ty::Struct(name)` → `name`,
/// `Ty::Sum(name)` → `name`. Outros → `""` (não valida).
fn ty_name(ty: &Ty) -> &str {
    match ty {
        Ty::Prim(PrimTy::Int) => "Int",
        Ty::Prim(PrimTy::Float) => "Float",
        Ty::Prim(PrimTy::Rational) => "Rational",
        Ty::Prim(PrimTy::Text) => "Text",
        Ty::Struct(name) => name,
        Ty::Sum(name) => name,
        _ => "",
    }
}
