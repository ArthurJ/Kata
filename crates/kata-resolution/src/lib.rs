//! Pass 0 + Pass 1: resolution.
//!
//! - Pass 0: popula TypeEnv com tipos declarados (`data` → Struct, `enum` → Sum)
//! - Pass 1: coleta assinaturas de funções `@ffi` e registra no DispatchTable
//!
//! Produz o `ResolvedModule` (imutável).

mod directives;
pub(crate) mod merge_imports;
pub(crate) mod module_loader;
mod pass0;
mod prelude_sigs;
mod type_resolve;
mod types;

pub use type_resolve::{collect_type_params, resolve_type_expr};
pub use types::*;

pub use module_loader::{ImportKind, ImportedModule, LoadError, ModuleLoader, filter_exports};
pub use merge_imports::merge_imports;

use directives::{extract_arg_keys, extract_site_when, extract_test_specs, extract_timer_spec};

use kata_ast::{Item, Module};
use kata_core::{Ty, TypeEnv};

/// Extrai a aridade padrão de cada nome de função a partir das assinaturas
/// resolvidas.
///
/// A aridade padrão é a aridade da **primeira** overload declarada para
/// cada nome. Usado pelo ciclo de dois passes (Fase 4) para alimentar
/// `parse_with_arity` no Pass 2.
pub fn extract_arities(signatures: &[Signature]) -> std::collections::HashMap<String, usize> {
    let mut arities = std::collections::HashMap::new();
    for sig in signatures {
        // insert_only: a primeira overload vence (ordem de declaração).
        match arities.entry(sig.name.clone()) {
            std::collections::hash_map::Entry::Occupied(_) => {
                // insert_only: a primeira overload vence (ordem de declaração).
                // Overloads com aridades diferentes são legítimas (dict dispatch);
                // a aridade padrão é a da primeira declaração, silenciosamente.
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(sig.param_types.len());
            }
        }
    }
    arities
}

/// Resolve um módulo: Pass 0 + Pass 1.
///
/// Usa `"__local__"` como origin para tipos definidos no módulo.
/// Para especificar o nome do módulo (importação), usar `resolve_with_origin`.
pub fn resolve(module: &Module) -> Result<ResolvedModule, Vec<ResolveError>> {
    resolve_with_origin(module, "__local__")
}

/// Resolve um módulo com origin explícita (nome do módulo).
///
/// `origin` é usado como `origin` em `TypeBinding`s para tipos definidos
/// neste módulo, permitindo desambiguação quando múltiplos módulos
/// definem tipos com o mesmo nome.
pub fn resolve_with_origin(
    module: &Module,
    origin: &str,
) -> Result<ResolvedModule, Vec<ResolveError>> {
    resolve_with_imports(module, origin, DirectiveRegistry::new())
}

/// Resolve um módulo com diretivas importadas pré-carregadas.
///
/// `imported_directives` contém diretivas de módulos importados que já foram
/// carregados e resolvidos. Estas diretivas são mescladas no `directive_registry`
/// antes da validação de `@nome` em Sig/ActionDecl, permitindo que o módulo
/// use `@trace_enter` quando `trace_enter` vem de um import.
///
/// Se `imported_directives` está vazio, comporta-se como `resolve_with_origin`.
pub fn resolve_with_imports(
    module: &Module,
    origin: &str,
    imported_directives: DirectiveRegistry,
) -> Result<ResolvedModule, Vec<ResolveError>> {
    resolve_inner(
        module,
        origin,
        imported_directives,
        kata_core::InterfaceRegistry::new(),
        &DirectiveRegistry::new(),
    )
}

/// Resolve um módulo com diretivas importadas e interfaces do prelude.
///
/// Igual a `resolve_with_imports`, mas pré-popula o `interface_registry`
/// com as interfaces do prelude. Isto é necessário para que tipos como
/// `msg :: SHOW` sejam resolvidos como `Ty::Interface("SHOW")` em vez de
/// `Ty::Var("SHOW")` quando o módulo do usuário não define a interface.
///
/// `prelude_directives` é o `DirectiveRegistry` do prelude (core.kata),
/// usado para **consulta** durante validação de `@nome` em Sig/ActionDecl.
/// As diretivas do prelude não são inseridas no registry do módulo — elas
/// são mescladas posteriormente em `merge_two`. Sem isto, `@trace` (definida
/// no stdlib) seria rejeitada como `unknown_directive` no resolve do usuário,
/// antes do merge trazer as declarations do prelude.
pub fn resolve_with_prelude(
    module: &Module,
    origin: &str,
    imported_directives: DirectiveRegistry,
    prelude_iface_reg: &kata_core::InterfaceRegistry,
    prelude_directives: &DirectiveRegistry,
) -> Result<ResolvedModule, Vec<ResolveError>> {
    resolve_inner(
        module,
        origin,
        imported_directives,
        prelude_iface_reg.clone(),
        prelude_directives,
    )
}

fn resolve_inner(
    module: &Module,
    origin: &str,
    imported_directives: DirectiveRegistry,
    prelude_iface_reg: kata_core::InterfaceRegistry,
    prelude_directives: &DirectiveRegistry,
) -> Result<ResolvedModule, Vec<ResolveError>> {
    let mut type_env = TypeEnv::new();
    // Unit é tipo primitivo da linguagem — sempre disponível no TypeEnv.
    type_env.define("Unit", Ty::Unit, origin);
    let mut signatures: Vec<Signature> = Vec::new();
    let mut functions: Vec<FunctionDef> = Vec::new();
    let mut actions: Vec<ActionDef> = Vec::new();
    let mut enum_registry = kata_core::EnumRegistry::new();
    let mut struct_registry = kata_core::StructRegistry::new();
    let mut refined_decls = Vec::new();
    let mut enum_pred_decls = Vec::new();
    // Pré-popula com interfaces do prelude para que `resolve_type_expr`
    // resolva `SHOW` como `Ty::Interface("SHOW")` em vez de `Ty::Var("SHOW")`.
    let mut interface_registry = prelude_iface_reg;
    let mut refines_registry = kata_core::RefinesRegistry::new();
    // Erros de validação de diretivas desconhecidas (coletado durante Pass 1).
    let mut errors: Vec<ResolveError> = Vec::new();

    // Pass 0: popula TypeEnv com tipos declarados
    pass0::run_pass0(
        &module.items,
        &mut type_env,
        &mut enum_registry,
        &mut struct_registry,
        &mut refined_decls,
        &mut enum_pred_decls,
        &mut interface_registry,
        &mut refines_registry,
        &mut signatures,
        &mut functions,
        &mut errors,
        origin,
    );

    // Pass 0.5: coleta diretivas customizadas (DirectiveDecl) no registry.
    // Antes do Pass 1 para que a validação de @nome em Sig/ActionDecl
    // possa consultar o registry.
    // Começa com diretivas importadas (se houver) e adiciona as locais.
    let mut directive_registry = imported_directives;
    for item in &module.items {
        if let Item::DirectiveDecl { name, args, body } = &item.node {
            match directives::extract_directive_spec(name, args, body.clone()) {
                Ok(def) => {
                    if let Err(e) = directive_registry.insert(def) {
                        errors.push(e);
                    }
                }
                Err(e) => errors.push(e),
            }
        }
    }

    // Validação 2.5.4: Target::Any não coexiste com específico para (nome, when).
    errors.extend(directive_registry.validate_any_conflicts());

    // Pass 1: coleta assinaturas de funções
    for item in &module.items {
        match &item.node {
            Item::Sig {
                name,
                params,
                ret,
                directives,
                body,
            } => {
                // Converte TypeExpr → Ty
                let param_types: Vec<Ty> = params
                    .iter()
                    .map(|t| resolve_type_expr(&t.node, &type_env, &interface_registry))
                    .collect();
                let return_type = resolve_type_expr(&ret.node, &type_env, &interface_registry);

                // Extrai metadados de diretivas
                let mut ffi_symbol = None;
                let mut is_associative = false;
                let mut associative_neutral = None;
                let mut is_commutative = false;
                let mut cache_strategy = None;

                for d in directives {
                    match d.name.as_str() {
                        "ffi" => {
                            if let Some(kata_ast::DirectiveArg::Expr(e)) = d.args.first()
                                && let kata_ast::Expr::TextLit { text } = &e.node
                            {
                                ffi_symbol = Some(text.clone());
                            }
                        }
                        "associative" => {
                            is_associative = true;
                            if let Some(kata_ast::DirectiveArg::Expr(e)) = d.args.first()
                                && let kata_ast::Expr::IntLit { text } = &e.node
                                && let Ok(n) = text.parse::<i64>()
                            {
                                associative_neutral = Some(n);
                            }
                        }
                        "commutative" => {
                            is_commutative = true;
                        }
                        "cache" => {
                            // @cache{strategy: "LRU"} — extrai a estratégia.
                            for arg in &d.args {
                                if let kata_ast::DirectiveArg::Named { key, value } = arg
                                    && key == "strategy"
                                    && let kata_ast::Expr::TextLit { text } = &value.node
                                {
                                    cache_strategy = Some(text.clone());
                                }
                            }
                        }
                        // Diretivas válidas em Sig mas sem processamento aqui.
                        "builtin" | "log" | "timer" => {}
                        // Diretiva customizada — validar contra o registry
                        // (local + prelude, para @trace do stdlib funcionar).
                        other
                            if directive_registry.contains_name(other)
                                || prelude_directives.contains_name(other) => {}
                        other => {
                            errors.push(ResolveError::UnknownDirective {
                                name: other.to_string(),
                                context: "sig",
                                item_name: name.clone(),
                            });
                        }
                    }
                }

                // Coleta type params (Ty::Var UPPER_CASE em params/ret).
                let type_params = collect_type_params(&param_types, &return_type);

                // Coleta diretivas customizadas (no registry) em ordem.
                // Valida Target: Sig é Function — diretiva com on: Target::Action
                // aplicada em Sig é erro.
                let custom_dirs: Vec<CustomDirectiveApp> = directives
                    .iter()
                    .filter(|d| {
                        directive_registry.contains_name(&d.name)
                            || prelude_directives.contains_name(&d.name)
                    })
                    .map(|d| CustomDirectiveApp {
                        name: d.name.clone(),
                        args: d.args.clone(),
                        arg_keys: extract_arg_keys(&d.args),
                        site_when: extract_site_when(&d.args),
                    })
                    .collect();
                for d in &custom_dirs {
                    if !directive_registry.has_compatible_target(&d.name, Target::Function)
                        && !prelude_directives.has_compatible_target(&d.name, Target::Function)
                    {
                        errors.push(ResolveError::DirectiveTargetMismatch {
                            name: d.name.clone(),
                            item_kind: "function".into(),
                            on: "Action".into(),
                        });
                    }
                }

                // Se tem corpo Kata (cláusulas lambda), preserva para o inference.
                if let Some(clauses) = body {
                    let timer = extract_timer_spec(directives, name, "sig", &mut errors);
                    functions.push(FunctionDef {
                        name: name.clone(),
                        param_types: param_types.clone(),
                        return_type: return_type.clone(),
                        clauses: clauses.clone(),
                        cache_strategy,
                        timer,
                        custom_directives: custom_dirs,
                    });
                }

                signatures.push(Signature {
                    name: name.clone(),
                    param_types,
                    return_type,
                    ffi_symbol,
                    is_associative,
                    associative_neutral,
                    is_action: false,
                    is_commutative,
                    type_params,
                });
            }
            Item::ActionDecl {
                name,
                params,
                param_names,
                param_defaults,
                ret,
                directives: action_dirs,
                body,
            } => {
                // Converte TypeExpr → Ty para os parâmetros e retorno.
                let param_types: Vec<Ty> = params
                    .iter()
                    .map(|t| resolve_type_expr(&t.node, &type_env, &interface_registry))
                    .collect();
                let return_type = resolve_type_expr(&ret.node, &type_env, &interface_registry);

                // Extrai ffi_symbol das diretivas da Action.
                let ffi_symbol = action_dirs.iter().find_map(|d| {
                    if d.name == "ffi"
                        && let Some(kata_ast::DirectiveArg::Expr(e)) = d.args.first()
                        && let kata_ast::Expr::TextLit { text } = &e.node
                    {
                        return Some(text.clone());
                    }
                    None
                });

                // Valida diretivas: @ffi, @test e @log são válidas em Actions.
                // Outras (@builtin, @commutative, @associative) pertencem a Sigs
                // ou Implements — erro se aparecerem em Action.
                for d in action_dirs {
                    match d.name.as_str() {
                        "ffi" | "test" | "log" => {}
                        // Diretiva customizada — validar contra o registry
                        // (local + prelude, para @trace do stdlib funcionar).
                        other
                            if directive_registry.contains_name(other)
                                || prelude_directives.contains_name(other) => {}
                        other => {
                            errors.push(ResolveError::UnknownDirective {
                                name: other.to_string(),
                                context: "action",
                                item_name: name.clone(),
                            });
                        }
                    }
                }

                // Coleta diretivas customizadas (no registry) em ordem.
                // Valida Target: ActionDecl é Action — diretiva com on: Target::Function
                // aplicada em Action é erro.
                let custom_dirs: Vec<CustomDirectiveApp> = action_dirs
                    .iter()
                    .filter(|d| {
                        directive_registry.contains_name(&d.name)
                            || prelude_directives.contains_name(&d.name)
                    })
                    .map(|d| CustomDirectiveApp {
                        name: d.name.clone(),
                        args: d.args.clone(),
                        arg_keys: extract_arg_keys(&d.args),
                        site_when: extract_site_when(&d.args),
                    })
                    .collect();
                for d in &custom_dirs {
                    if !directive_registry.has_compatible_target(&d.name, Target::Action)
                        && !prelude_directives.has_compatible_target(&d.name, Target::Action)
                    {
                        errors.push(ResolveError::DirectiveTargetMismatch {
                            name: d.name.clone(),
                            item_kind: "action".into(),
                            on: "Function".into(),
                        });
                    }
                }

                // Extrai casos de teste das diretivas @test.
                // @test("desc") — forma curta: desc é o primeiro posicional.
                // @test{desc: "...", args: (1,2), timeout: 5000, expects: "Panic: msg"}
                //   — forma dict: chaves nomeadas.
                let tests = extract_test_specs(action_dirs, name, &mut errors);

                // Se tem @ffi e body vazio → Action FFI builtin.
                // Produz uma Signature com is_action = true para o DispatchTable.
                // Não produz ActionDef (sem corpo Kata para o inference processar).
                if ffi_symbol.is_some() && body.is_empty() {
                    signatures.push(Signature {
                        name: name.clone(),
                        param_types: param_types.clone(),
                        return_type: return_type.clone(),
                        ffi_symbol,
                        is_associative: false,
                        associative_neutral: None,
                        is_action: true,
                        is_commutative: false,
                        type_params: vec![],
                    });
                } else {
                    // Action com corpo Kata — produz ActionDef para o inference.
                    actions.push(ActionDef {
                        name: name.clone(),
                        param_types,
                        param_names: param_names.clone(),
                        param_defaults: param_defaults.clone(),
                        return_type,
                        body: body.clone(),
                        tests,
                        custom_directives: custom_dirs,
                    });
                }
            }
            _ => {}
        }
    }

    // Validação D12 (removida): directive e action com mesmo nome podem
    // coexistir — `@log{...}` (diretiva) e `log!(...)` (action) são
    // sintaticamente distintas (`@` vs `!`). Não há ambiguidade.

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
        functions,
        actions,
        directive_registry,
    })
}

pub use prelude_sigs::load_prelude;

/// Combina dois ResolvedModules (prelude + módulo) em um único.
///
/// Usado pelo ModuleLoader para injetar o prelude em sub-módulos.
/// O driver tem sua própria versão (`merge_resolved`) que faz o mesmo
/// mas com acesso a imports; esta é a versão simples sem imports.
pub fn merge_two(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);

    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);

    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);

    let mut refined_decls = prelude.refined_decls;
    refined_decls.extend(user.refined_decls);

    let mut enum_pred_decls = prelude.enum_pred_decls;
    enum_pred_decls.extend(user.enum_pred_decls);

    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);

    let mut refines_registry = prelude.refines_registry;
    refines_registry.merge(user.refines_registry);

    let mut functions = prelude.functions;
    let user_fn_names: std::collections::HashSet<&str> =
        user.functions.iter().map(|f| f.name.as_str()).collect();
    functions.retain(|f| !user_fn_names.contains(f.name.as_str()));
    functions.extend(user.functions);

    let mut actions = prelude.actions;
    let user_action_names: std::collections::HashSet<&str> =
        user.actions.iter().map(|a| a.name.as_str()).collect();
    actions.retain(|a| !user_action_names.contains(a.name.as_str()));
    actions.extend(user.actions);

    // Validar impls após merge do prelude — antes disso, interfaces do
    // prelude (NUM, SHOW, etc.) não estavam visíveis no resolve do módulo.
    for warning in interface_registry.validate_impls_after_merge() {
        eprintln!("[resolution] warning: {warning}");
    }

    // Diretivas: mescla preservando overloads por (when, on).
    // Diferente de actions (nomes se substituem), diretivas com mesmo nome
    // coexistem quando (when, on) diferem.
    let mut directive_registry = prelude.directive_registry;
    let merge_errors = directive_registry.merge(user.directive_registry);
    for e in merge_errors {
        eprintln!("[resolution] warning: {e}");
    }

    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
        functions,
        actions,
        directive_registry,
    }
}
