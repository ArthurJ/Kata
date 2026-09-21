//! Pass 0 + Pass 1: resolution.
//!
//! - Pass 0: popula TypeEnv com tipos declarados (`data` → Struct, `enum` → Sum)
//! - Pass 1: coleta assinaturas de funções `@ffi` e registra no DispatchTable
//!
//! Produz o `ResolvedModule` (imutável).

mod directives;
pub(crate) mod embed;
mod families;
pub(crate) mod ident_collector;
pub(crate) mod merge_imports;
pub(crate) mod module_loader;
mod pass0;
mod type_resolve;
mod types;

pub use embed::{EmbedError, resolve_embeds};
pub use families::expand_family_signatures;
pub use type_resolve::{collect_type_params, resolve_type_expr};
pub use types::*;

pub use merge_imports::merge_imports;
pub use module_loader::{ImportedModule, LoadError, ModuleLoader};

use directives::{
    extract_arg_keys, extract_pragma_test_specs, extract_site_when, extract_test_specs,
    extract_timer_spec,
};

use kata_ast::{Item, Module};
use kata_core::{Ty, TypeEnv};
use kata_core::struct_registry::StructRegistry;

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

/// Extrai aridades de construtores de `data` (smart constructors) a partir
/// do `struct_registry`.
///
/// Construtores são sintetizados no inference (`constructors.rs`), mas o
/// parser precisa das aridades no Pass 2 — antes do inference. Esta função
/// espelha a lógica de filtragem de `synthesize_constructors` para que o
/// parser arity-aware trate construtores de `data` igual às funções comuns:
/// `Complex 5.0 5.0` coleta exatamente 2 argumentos (não greedy).
///
/// Refined types com predicates são incluídos — o construtor falível
/// sintetizado por `constructors_refined` tem a mesma aridade (1 parâmetro
/// do tipo base). Famílias polimórficas (`is_instance_of`) não são
/// construíveis diretamente e são puladas.
pub fn extract_constructor_arities(
    struct_registry: &StructRegistry,
) -> std::collections::HashMap<String, usize> {
    let mut arities = std::collections::HashMap::new();
    for name in struct_registry.names() {
        let Some(info) = struct_registry.get(name) else {
            continue;
        };
        // Família polimórfica — não construtível diretamente.
        if info.is_instance_of.is_some() {
            continue;
        }
        // Struct sem campos (tipo opaco) — não ganha construtor.
        // Alias de primitivo opaco (fields vazio) ganha construtor identity
        // de aridade 1, mas só se alias_of.is_some(). Struct sem campos e
        // sem alias não tem construtor algum.
        let arity = if info.fields.is_empty() {
            if info.alias_of.is_some() {
                // Alias de tipo opaco: construtor identity, aridade 1.
                1
            } else {
                continue;
            }
        } else {
            info.fields.len()
        };
        // insert_only: não sobrescreve aridade de função já declarada.
        arities.entry(name.to_string()).or_insert(arity);
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
/// use `@log` quando `log` vem de um import.
///
/// Se `imported_directives` está vazio, comporta-se como `resolve_with_origin`.
pub(crate) fn resolve_with_imports(
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
        None,
        None,
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
/// são mescladas posteriormente em `merge_two`. Sem isto, `@log` (definida
/// no stdlib) seria rejeitada como `unknown_directive` no resolve do usuário,
/// antes do merge trazer as declarations do prelude.
pub fn resolve_with_prelude(
    module: &Module,
    origin: &str,
    imported_directives: DirectiveRegistry,
    prelude_iface_reg: &kata_core::InterfaceRegistry,
    prelude_directives: &DirectiveRegistry,
    prelude_type_graph: Option<&kata_core::TypeGraph>,
    prelude_type_env: Option<&TypeEnv>,
) -> Result<ResolvedModule, Vec<ResolveError>> {
    resolve_inner(
        module,
        origin,
        imported_directives,
        prelude_iface_reg.clone(),
        prelude_directives,
        prelude_type_graph,
        prelude_type_env,
    )
}

fn resolve_inner(
    module: &Module,
    origin: &str,
    imported_directives: DirectiveRegistry,
    prelude_iface_reg: kata_core::InterfaceRegistry,
    prelude_directives: &DirectiveRegistry,
    prelude_type_graph: Option<&kata_core::TypeGraph>,
    prelude_type_env: Option<&TypeEnv>,
) -> Result<ResolvedModule, Vec<ResolveError>> {
    // Se o prelude_type_env está disponível, usa-o como parent do TypeEnv
    // do módulo do usuário. Isto permite que `resolve_type_expr` encontre
    // tipos definidos na stdlib (ex: `Encoding` → `Sum("Encoding")`) em
    // anotações de tipo do usuário, ANTES do merge_two que acontece depois.
    // Sem isto, tipos da stdlib em assinaturas do usuário (ex:
    // `foo :: Result::(Int, Encoding) => Text`) caem para `Struct(Plain(...))`
    // porque o lookup no TypeEnv local retorna None.
    let mut type_env = match prelude_type_env {
        Some(parent) => TypeEnv::with_parent(parent.clone()),
        None => TypeEnv::new(),
    };
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

    // Injeção estrutural: módulo stdio exporta __stdin__/__stdout__/__stderr__
    // como valores Ty::File. Não são declarados no .kata — são injetados aqui
    // porque FD 0/1/2 são fatos do mundo, não computações. O codegen lowera
    // estes idents para chamadas FFI (kata_rt_stdin/stdout/stderr).
    if origin == "stdio" {
        type_env.define("__stdin__", Ty::File, "stdio");
        type_env.define("__stdout__", Ty::File, "stdio");
        type_env.define("__stderr__", Ty::File, "stdio");
    }

    // Constrói o TypeGraph a partir dos registries populados no Pass 0.
    // Para módulos do usuário, o prelude_iface_reg já traz interfaces do
    // prelude (SHOW, NUM, etc.). Se `prelude_type_graph` está disponível,
    // faz merge antes do Pass 1 — assim `resolve_type_expr` conhece
    // structs/enums do prelude (NonZero, Result) durante a resolução de
    // assinaturas, sem esperar por `merge_two`.
    let mut type_graph = kata_core::TypeGraphBuilder {
        struct_reg: &struct_registry,
        enum_reg: &enum_registry,
        iface_reg: &interface_registry,
        refines_reg: &refines_registry,
    }
    .build(origin);
    if let Some(prelude_tg) = prelude_type_graph {
        type_graph.merge(prelude_tg);
    }

    // Pass 0.5: coleta diretivas customizadas (DirectiveDecl) no registry.
    // Antes do Pass 1 para que a validação de @nome em Sig/ActionDecl
    // possa consultar o registry.
    // Começa com diretivas importadas (se houver) e adiciona as locais.
    let mut directive_registry = imported_directives;
    for item in &module.items {
        if let Item::DirectiveDecl { name, args, body } = &item.item.node {
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
        match &item.item.node {
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
                    .map(|t| {
                        resolve_type_expr(
                            &t.node,
                            &type_env,
                            &interface_registry,
                            &struct_registry,
                            Some(&type_graph),
                        )
                    })
                    .collect();
                let return_type = resolve_type_expr(
                    &ret.node,
                    &type_env,
                    &interface_registry,
                    &struct_registry,
                    Some(&type_graph),
                );

                // Extrai metadados de diretivas
                let mut ffi_symbol = None;
                let mut is_associative = false;
                let mut associative_neutral = None;
                let mut is_commutative = false;
                let mut cache_strategy = None;
                let mut cache_capacity = None;

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
                            // @cache presente — ativa com defaults mesmo sem args.
                            cache_strategy = Some("LRU".to_string());
                            cache_capacity = Some(256);
                            for arg in &d.args {
                                if let kata_ast::DirectiveArg::Named { key, value } = arg {
                                    match key.as_str() {
                                        "strategy" => {
                                            if let kata_ast::Expr::TextLit { text } = &value.node {
                                                match text.as_str() {
                                                    "LRU" | "FIFO" | "MRU" | "LFU" => {
                                                        cache_strategy = Some(text.clone());
                                                    }
                                                    _ => errors.push(
                                                        ResolveError::UnknownCacheStrategy {
                                                            strategy: text.clone(),
                                                            item_name: name.clone(),
                                                        },
                                                    ),
                                                }
                                            }
                                        }
                                        "capacity" => {
                                            if let kata_ast::Expr::IntLit { text } = &value.node
                                                && let Ok(n) = text.parse::<i64>()
                                            {
                                                if n <= 0 {
                                                    errors.push(
                                                        ResolveError::CacheCapacityInvalid {
                                                            value: n,
                                                            item_name: name.clone(),
                                                        },
                                                    );
                                                } else {
                                                    cache_capacity = Some(n);
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        // Diretivas válidas em Sig mas sem processamento aqui.
                        "builtin" | "log" | "timer" => {}
                        // Diretiva customizada — validar contra o registry
                        // (local + prelude, para @log do stdlib funcionar).
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
                        cache_capacity,
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
                    param_names: vec![],
                    param_defaults: vec![],
                });
            }
            Item::ActionDecl {
                name,
                params,
                param_names,
                param_defaults,
                ret,
                directives: action_dirs,
                pragmas: action_pragmas,
                body,
            } => {
                // Converte TypeExpr → Ty para os parâmetros e retorno.
                let param_types: Vec<Ty> = params
                    .iter()
                    .map(|t| {
                        resolve_type_expr(
                            &t.node,
                            &type_env,
                            &interface_registry,
                            &struct_registry,
                            Some(&type_graph),
                        )
                    })
                    .collect();
                let return_type = resolve_type_expr(
                    &ret.node,
                    &type_env,
                    &interface_registry,
                    &struct_registry,
                    Some(&type_graph),
                );

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
                        // (local + prelude, para @log do stdlib funcionar).
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
                let mut tests = extract_test_specs(action_dirs, name, &mut errors);

                // Extrai casos de teste dos pragmas #!test anexados à action.
                // #!test("desc") e #!test{desc, args, timeout} — marker puro,
                // sem expects/policy (esses permanecem @test{expects}).
                tests.extend(extract_pragma_test_specs(action_pragmas));

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
                        param_names: param_names.clone(),
                        param_defaults: param_defaults.clone(),
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
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
        type_graph,
        functions,
        actions,
        directive_registry,
        embed_dependencies: Vec::new(),
    })
}

/// Carrega a stdlib (core → core_internals) via `ModuleLoader` embedded.
///
/// Carrega a stdlib embedded para testes e callers que precisam do
/// `ResolvedModule` não-filtrado da stdlib. Carrega `["stdlib", "core"]`
/// (não `mod.kata`) para preservar tipos primitivos não-qualificados
/// (`Int`, `Float`, etc.) no `TypeEnv`.
pub fn load_stdlib_for_tests() -> Result<ResolvedModule, Vec<ResolveError>> {
    use std::path::Path;
    use std::sync::OnceLock;

    static STDLIB: OnceLock<Result<ResolvedModule, Vec<ResolveError>>> = OnceLock::new();
    STDLIB
        .get_or_init(|| {
            let mut loader = module_loader::ModuleLoader::new(Vec::new());
            loader
                .load(&["stdlib".into(), "core".into()], Path::new("."))
                .map(|arc| (*arc).clone())
                .map_err(|e| match e {
                    module_loader::LoadError::Resolve(errors) => errors,
                    other => vec![ResolveError::UnknownFfi {
                        name: format!("erro ao carregar stdlib: {other}"),
                    }],
                })
        })
        .clone()
}

/// Combina dois ResolvedModules (prelude + módulo) em um único.
///
/// Usado pelo ModuleLoader para injetar o prelude em sub-módulos.
/// O driver tem sua própria versão (`merge_resolved`) que faz o mesmo
/// mas com acesso a imports; esta é a versão simples sem imports.
pub fn merge_two(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);

    let mut internal_signatures = prelude.internal_signatures;
    internal_signatures.extend(user.internal_signatures);

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

    // Merge do TypeGraph: prelude + user. Nós locais prevalecem (shadow).
    let mut type_graph = prelude.type_graph;
    type_graph.merge(&user.type_graph);

    let mut functions = prelude.functions;
    // Remove prelude functions whose (name, param_types, return_type) key
    // is redefined by the user. Overloads with different param types coexist.
    let user_fn_keys: std::collections::HashSet<(&str, &[Ty], &Ty)> = user
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.param_types.as_slice(), &f.return_type))
        .collect();
    functions.retain(|f| {
        !user_fn_keys.contains(&(f.name.as_str(), f.param_types.as_slice(), &f.return_type))
    });
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

    // Extensão de famílias polimórficas: para cada `T implements IFACE` do
    // usuário, estender todas as famílias sobre IFACE com a instância Fam::T.
    // Isto corrige o bug onde `data MyNum; MyNum implements NUM` falha porque
    // NonZero::MyNum nunca foi registrada (a família foi expandida eagerly
    // em pass0 com apenas os implementors do prelude).
    families::extend_families_for_implementors(
        &mut struct_registry,
        &mut refined_decls,
        &interface_registry,
        &mut type_graph,
    );

    // Re-instanciar Family → Instance nas signatures e functiondefs do
    // usuário que não puderam ser instanciadas no pass0 (porque a instância
    // da família ainda não existia). Agora que extend_families_for_implementors
    // registrou as instâncias faltantes, re-aplicar instantiate_family_for_concrete
    // com o concrete_type correto (o tipo que implementa a interface).
    families::reinstantiate_family_params(
        &mut signatures,
        &mut functions,
        &interface_registry,
        &struct_registry,
    );

    // Diretivas: mescla preservando overloads por (when, on).
    // Diferente de actions (nomes se substituem), diretivas com mesmo nome
    // coexistem quando (when, on) diferem.
    let mut directive_registry = prelude.directive_registry;
    let merge_errors = directive_registry.merge(user.directive_registry);
    for e in merge_errors {
        eprintln!("[resolution] warning: {e}");
    }

    let mut embed_dependencies = prelude.embed_dependencies;
    embed_dependencies.extend(user.embed_dependencies);

    ResolvedModule {
        type_env,
        signatures,
        internal_signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
        type_graph,
        functions,
        actions,
        directive_registry,
        embed_dependencies,
    }
}

/// Gate 1: regra do órfão.
///
/// Um `implements` é órfão quando nem o tipo nem a interface são
/// definidos no mesmo módulo onde o implements foi declarado.
/// Isto viola a regra do órfão (análoga à de Rust): ou o tipo OU
/// a interface deve ser local ao módulo do implements.
///
/// Consulta `origins_of` em struct_registry, enum_registry e
/// interface_registry para determinar as origins de cada lado.
pub fn validate_orphan_rule(
    interface_registry: &kata_core::InterfaceRegistry,
    struct_registry: &kata_core::StructRegistry,
    enum_registry: &kata_core::EnumRegistry,
) -> Vec<ResolveError> {
    let mut errors = Vec::new();

    for entry in interface_registry.impls_view() {
        // Impls sintéticos (show_synthesis, etc.) têm span synthetic e
        // origin interna — não estão sujeitos à regra do órfão.
        if entry.origin == "__synthetic__" || entry.origin == "core" {
            continue;
        }

        // Origins onde o tipo é definido.
        let type_origins: Vec<String> = struct_registry
            .origins_of(&entry.type_name)
            .into_iter()
            .map(|s| s.to_string())
            .collect();

        // Se o tipo não é struct, tentar enum.
        let type_origins = if type_origins.is_empty() {
            enum_registry
                .origins_of(&entry.type_name)
                .into_iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        } else {
            type_origins
        };

        // Origins onde a interface é definida.
        let iface_origins: Vec<String> = interface_registry
            .origins_of(&entry.interface_name)
            .into_iter()
            .map(|s| s.to_string())
            .collect();

        // Se o tipo OU a interface é local ao módulo do implements, OK.
        let impl_origin = &entry.origin;
        let type_is_local = type_origins.iter().any(|o| o == impl_origin);
        let iface_is_local = iface_origins.iter().any(|o| o == impl_origin);

        if !type_is_local && !iface_is_local {
            errors.push(ResolveError::OrphanImpl {
                type_name: entry.type_name.clone(),
                interface_name: entry.interface_name.clone(),
                impl_origin: impl_origin.clone(),
                type_origin: type_origins.first().cloned().unwrap_or_default(),
                iface_origin: iface_origins.first().cloned().unwrap_or_default(),
                span: entry.span.into(),
            });
        }
    }

    errors
}

/// Gate 2: implementação incompleta de interface.
///
/// Para cada `T implements IFACE`, verifica que todos os métodos
/// **obrigatórios** (sem `default_body`) de IFACE e seus supertraits
/// têm um método com o mesmo nome definido no impl. Métodos com
/// `default_body` contam como cobertos (o pass0b sintetiza o corpo).
///
/// Retorna `Vec<ResolveError>` com `IncompleteInterface` para cada impl
/// que deixou métodos obrigatórios faltando. O erro é controlável via
/// `#!allow type.incomplete_interface` (ver `filter_errors` no pipeline).
pub fn validate_incomplete_interfaces(
    interface_registry: &kata_core::InterfaceRegistry,
) -> Vec<ResolveError> {
    let mut errors = Vec::new();

    for entry in interface_registry.impls_view() {
        // Só valida impls declarados no módulo do usuário. Impls
        // importados (origin "complex", "math", etc.) e sintéticos
        // são validados em seu módulo de origem.
        if entry.origin != "__local__" {
            continue;
        }

        // Métodos definidos pelo usuário EM QUALQUER impl do mesmo tipo.
        // Ex: `MyNum implements NUM` define 8 métodos de NUM, e
        // `MyNum implements EQ` define `=` e `!=`. NUM tem supertrait EQ,
        // então o check de NUM precisa ver os métodos de EQ também.
        let defined_names: std::collections::HashSet<&str> = interface_registry
            .get_impls_for_type(&entry.type_name)
            .iter()
            .flat_map(|e| e.methods.iter().map(|m| m.name.as_str()))
            .collect();

        // Coletar métodos obrigatórios faltando da interface + supertraits.
        let missing = collect_missing_methods(
            interface_registry,
            &entry.interface_name,
            &defined_names,
            &entry.type_name,
        );

        if !missing.is_empty() {
            errors.push(ResolveError::IncompleteInterface {
                type_name: entry.type_name.clone(),
                interface_name: entry.interface_name.clone(),
                missing,
                span: entry.span.into(),
            });
        }
    }

    errors
}

/// Percorre a interface e seus supertraits recursivamente, coletando
/// métodos obrigatórios (sem `default_body`) que não estão em
/// `defined_names`. Retorna `Vec<(name, signature)>` onde signature
/// é a string formatada com `Self` substituído pelo nome do tipo.
fn collect_missing_methods(
    registry: &kata_core::InterfaceRegistry,
    iface_name: &str,
    defined_names: &std::collections::HashSet<&str>,
    concrete_ty_name: &str,
) -> Vec<(String, String)> {
    let mut missing = Vec::new();
    let mut visited = std::collections::HashSet::new();
    collect_missing_methods_inner(
        registry,
        iface_name,
        defined_names,
        &mut missing,
        &mut visited,
        concrete_ty_name,
    );
    missing
}

fn collect_missing_methods_inner(
    registry: &kata_core::InterfaceRegistry,
    iface_name: &str,
    defined_names: &std::collections::HashSet<&str>,
    missing: &mut Vec<(String, String)>,
    visited: &mut std::collections::HashSet<String>,
    concrete_ty_name: &str,
) {
    if !visited.insert(iface_name.to_string()) {
        return;
    }

    let Some(info) = registry.get_interface(iface_name) else {
        return;
    };

    let concrete_ty = kata_core::Ty::Struct(kata_core::StructKey::Plain(
        concrete_ty_name.to_string(),
    ));

    for sig in &info.signatures {
        // Métodos com default_body contam como cobertos.
        if sig.default_body.is_some() {
            continue;
        }
        if !defined_names.contains(sig.name.as_str()) {
            // Formatar signature com Self substituído: "name :: params => ret"
            let params: Vec<String> = sig
                .params
                .iter()
                .map(|t| t.substitute_self(&concrete_ty).to_string())
                .collect();
            let ret = sig.ret.substitute_self(&concrete_ty).to_string();
            let params_str = params.join(" ");
            let sig_str = if params_str.is_empty() {
                format!("{} => {}", sig.name, ret)
            } else {
                format!("{} :: {} => {}", sig.name, params_str, ret)
            };
            missing.push((sig.name.clone(), sig_str));
        }
    }

    // Recursão em supertraits.
    for st in &info.supertraits {
        collect_missing_methods_inner(
            registry,
            st,
            defined_names,
            missing,
            visited,
            concrete_ty_name,
        );
    }
}
