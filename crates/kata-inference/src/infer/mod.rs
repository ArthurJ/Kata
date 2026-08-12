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
mod apply_dispatch;
mod apply_lambda;
mod apply_len_tuple;
mod ascription;
mod captures;
mod collections;
mod collections_hof;
mod const_eval;
mod constness;
mod constructors;
mod constructors_enum_pred;
mod constructors_refined;
mod cross_process;
mod csp;
mod csp_builtins;
mod cycle;
mod dict_set;
mod dot_access;
mod expr;
mod format_synthesis;
mod free_vars;
mod function_infer;
pub(crate) mod generics;
pub(crate) mod helpers;
mod iface_dispatch;
mod lambda;
mod log_builtins;
mod log_synthesis;
mod log_template;
mod partial_dispatch;
mod recursion;
mod refined_builders;
mod show_synthesis;
mod show_synthesis_helpers;
mod show_synthesis_list;
mod sugar;
mod timer_builtins;
mod variant;
mod variant_construct;
mod variant_qual;
mod walk;

use action_infer::infer_action;
use function_infer::{infer_named_function, ty_name};
use kata_ast::{Item, Module, Spanned};
use kata_core::dispatch::OverloadInfo;
use kata_core::ty::Ty;
use kata_diagnostics::MiddleError;
use kata_resolution::ResolvedModule;
use kata_resolution::collect_type_params;

use crate::desugar;
use crate::typed::{TypedAction, TypedExpr, TypedFunction, TypedModule};

use self::expr::{InferCtx, infer_expr};
use self::helpers::{item_span_or_synthetic, populate_dispatch_table};

/// Infere o tipo de um módulo completo.
///
/// Pipeline: popula DispatchTable → processa funções nomeadas → infere entry point.
/// Retorna `TypedModule` ou o primeiro erro de typeck encontrado.
pub fn infer_module(
    module: &Module,
    resolved: &ResolvedModule,
) -> Result<TypedModule, MiddleError> {
    // Reseta o contador de type vars de canal — cada channel!() precisa
    // de um nome único para que a unificação não colida entre canais.
    csp_builtins::reset_channel_type_var_counter();

    // Side table para use-site inference de lambdas deferidos.
    // Quando `let f := lambda a b: - a b` falha (partial dispatch ambíguo),
    // o lambda é guardado aqui. Quando `f 5 3` é aplicado, infer_apply
    // consulta esta table e re-inere o lambda com os arg types reais.
    let deferred_lambdas = expr::DeferredLambdaTable::default();

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

    // 2a. Pré-registra constants de módulo no TypeEnv ANTES de inferir
    //     funções nomeadas. Funções nomeadas podem referenciar constants
    //     no corpo — sem o pré-registro, UnboundName.
    //     Inferência dedicada (C3): não envolve em Expr::Let — chama
    //     infer_expr diretamente no value e registra no type_env com
    //     origin __module__. Validações de constness (lambda, pureza,
    //     comptime-availability) são feitas aqui, não no comptime pass.
    let mut constant_typed_values: Vec<(String, Spanned<TypedExpr>)> = Vec::new();
    let mut seen_constant_names: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for item in &module.items {
        if let Item::ConstantDecl { name, value } = &item.node {
            // Constants são imutáveis por design — redefinir o mesmo nome é erro.
            if !seen_constant_names.insert(name.clone()) {
                return Err(MiddleError::DuplicateConstant {
                    name: name.clone(),
                    span: value.span.into(),
                });
            }
            // Colisão com função/action: o nome já existe no dispatch_table
            // (populado no passo 1 com functions e actions).
            if dispatch_table.has_function(name) {
                return Err(MiddleError::ConstantNameCollision {
                    name: name.clone(),
                    span: value.span.into(),
                });
            }
            let desugared = desugar::desugar(value);
            let ctx = InferCtx {
                table: &dispatch_table,
                enum_registry: &resolved.enum_registry,
                struct_registry: &resolved.struct_registry,
                refined_decls: &resolved.refined_decls,
                interface_registry: &interface_registry,
                refines_registry: &resolved.refines_registry,
                ret_ty: None,
                in_loop: false,
                deferred_lambdas: &deferred_lambdas,
            };
            // Inferência direta do value (sem wrapping em Expr::Let).
            let typed_value =
                infer_expr(&desugared.node, &desugared.span, &mut type_env, &ctx, false)?;

            // ── Validação de constness (C3): detectar lambda aqui, não
            //    no comptime pass. Pureza e comptime-availability
            //    continuam no comptime pass (dependem de contexto de
            //    avaliação — alguns testes não rodam comptime pass). ──
            // 1. Lambda como value → ConstantLambda (PRD §3.7).
            //    Lambdas e sections (que desugar para lambda) não são
            //    permitidos em `constant`. Para funções, use sintaxe
            //    de função nomeada (f :: T => T / lambda ...).
            constness::check_constant_lambda(name, &typed_value, value.span)?;

            // Registrar no type_env com origin __module__.
            if name != "_" {
                type_env.define(name, typed_value.ty.clone(), "__module__");
            }

            constant_typed_values.push((name.clone(), Spanned::new(typed_value, value.span)));
        }
    }

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
            deferred_lambdas: &deferred_lambdas,
        };
        let typed_func = infer_named_function(func_def, &ctx, &type_env)?;
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
            deferred_lambdas: &deferred_lambdas,
        };
        let typed_action = infer_action(action_def, &ctx, &type_env)?;
        typed_actions.push(typed_action);
    }

    // 3b. verifica que nenhuma Action é recursiva.
    //     Actions executam em fibers com stack fixa; recursão estouraria.
    recursion::check_action_recursion(&typed_actions)?;

    // 4. Percorre items — infere cada EntryExpr em sequência.
    //    O último vira o entry point; os anteriores viram pre_entry
    //    (lowerados em sequência pelo codegen, compartilhando var_map).
    let mut pre_entry: Vec<Spanned<TypedExpr>> = Vec::new();
    let mut constants: Vec<Spanned<TypedExpr>> = Vec::new();
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
                    deferred_lambdas: &deferred_lambdas,
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
            | Item::ExportDecl { .. }
            | Item::DirectiveDecl { .. } => {
                // Já processado no resolution/inference de funções nomeadas.
                // Interfaces/implements/import/export são processados
                // (resolution) — o inference não os processa.
                // DirectiveDecl é processado no resolution (DirectiveRegistry)
                // e consumido pelo desugar_directives antes do inference.
            }
            Item::ConstantDecl { name, value } => {
                // Pré-processado no passo 2a. Aqui produzimos um
                // ConstantBinding na coleção constants (não pre_entry).
                // O comptime pass avalia o RHS via JIT-and-execute e
                // substitui por literal/HeapSnapshot. Se o RHS não é
                // comptime-available (ex: lambda), erro de compilação.
                let typed_value = constant_typed_values
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, tv)| tv.node.clone())
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: format!("constant {name}"),
                        span: value.span.into(),
                        suggestion: None,
                    })?;

                let binding = TypedExpr {
                    span: value.span,
                    ty: typed_value.ty.clone(),
                    tail_pos: false,
                    escape: kata_core::escape::EscapeTarget::Local,
                    kind: crate::typed::TypedExprKind::ConstantBinding {
                        name: name.clone(),
                        value: Box::new(Spanned::new(typed_value, value.span)),
                    },
                };
                constants.push(Spanned::new(binding, value.span));
            }
            Item::ActionDecl { .. } => {
                // Já processado no inference de Actions (abaixo).
            }
        }
    }

    let entry = entry_expr.ok_or_else(|| MiddleError::UnboundName {
        name: "<entry point>".into(),
        span: item_span_or_synthetic(&module.items),
        suggestion: None,
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
        struct_registry: resolved.struct_registry.clone(),
        snapshots: Vec::new(),
        refined_decls: resolved.refined_decls.clone(),
        constants,
    };

    // Coleta captures (free variables) de cada Closure.
    // Percorre a TAST já construída e muta in-place os campos `captures`.
    captures::run(&mut typed_module);

    // Marca canais que fluem para spawn! como cross_process.
    cross_process::run(&mut typed_module);

    Ok(typed_module)
}

/// Envolve o entry point de um `TypedModule` com `show` para que o driver
/// possa imprimir tipos compostos (List, Tuple, Struct, Sum) como Text.
///
/// Se o tipo do entry point é um tipo composto, substitui o entry por
/// `show <entry>` (Closure com callee = Ident("show"), ffi_symbol = None).
/// O monomorphizador resolve o overload genérico e instancia para o tipo
/// concreto. Primitivos (Int, Float, Text, Rational, Boolean, Unit) não
/// são afetados — já têm type tags no driver.
pub fn wrap_entry_with_show(typed: &mut TypedModule) {
    use crate::typed::TypedExprKind;
    use kata_core::escape::EscapeTarget;

    let entry_ty = typed.entry.node.ty.clone();
    let needs_wrap = match &entry_ty {
        Ty::Prim(_) | Ty::Unit => false,
        Ty::Sum(name) if name == "Boolean" => false,
        Ty::List(_) | Ty::Tuple(_) | Ty::Struct(_) | Ty::Sum(_) => true,
        _ => false,
    };
    if !needs_wrap {
        return;
    }

    let entry_span = typed.entry.span;
    let entry_inner = std::mem::replace(
        &mut typed.entry,
        Spanned::new(
            TypedExpr {
                span: entry_span,
                ty: entry_ty.clone(),
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Unit,
            },
            entry_span,
        ),
    );

    // Constrói `show <entry>` como Closure. O monomorphizador encontra
    // o overload `show` na DispatchTable, instancia para o tipo concreto,
    // e reescreve o callee para o nome da instância.
    let callee = TypedExpr {
        span: entry_span,
        ty: Ty::Function(vec![entry_ty.clone()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "show".to_string(),
        },
    };
    typed.entry = Spanned::new(
        TypedExpr {
            span: entry_span,
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Caller,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, entry_span)),
                args: vec![entry_inner],
                ffi_symbol: None,
            },
        },
        entry_span,
    );
}
