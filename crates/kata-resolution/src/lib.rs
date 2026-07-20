//! Pass 0 + Pass 1: resolution.
//!
//! - Pass 0: popula TypeEnv com tipos declarados (`data` → Struct, `enum` → Sum)
//! - Pass 1: coleta assinaturas de funções `@ffi` e registra no DispatchTable
//!
//! Produz o `ResolvedModule` (imutável).

pub(crate) mod module_loader;
mod pass0;
mod prelude_sigs;
mod type_resolve;
mod types;

pub use type_resolve::collect_type_params;
pub use types::*;

use kata_ast::{Item, Module};
use kata_core::{Ty, TypeEnv};
use type_resolve::resolve_type_expr;

/// Resolve um módulo: Pass 0 + Pass 1.
pub fn resolve(module: &Module) -> Result<ResolvedModule, Vec<ResolveError>> {
    let mut type_env = TypeEnv::new();
    // Unit é tipo primitivo da linguagem — sempre disponível no TypeEnv.
    type_env.define("Unit", Ty::Unit);
    let mut signatures: Vec<Signature> = Vec::new();
    let mut functions: Vec<FunctionDef> = Vec::new();
    let mut actions: Vec<ActionDef> = Vec::new();
    let mut enum_registry = kata_core::EnumRegistry::new();
    let mut struct_registry = kata_core::StructRegistry::new();
    let mut refined_decls = Vec::new();
    let mut enum_pred_decls = Vec::new();
    let mut interface_registry = kata_core::InterfaceRegistry::new();
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
        &mut signatures,
        &mut functions,
        &mut errors,
    );

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
                        // Diretivas válidas em Sig mas sem processamento aqui.
                        "builtin" | "log" => {}
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

                // Se tem corpo Kata (cláusulas lambda), preserva para o inference.
                if let Some(clauses) = body {
                    let log = extract_log_spec(directives, name, "sig", &mut errors);
                    functions.push(FunctionDef {
                        name: name.clone(),
                        param_types: param_types.clone(),
                        return_type: return_type.clone(),
                        clauses: clauses.clone(),
                        log,
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
                        other => {
                            errors.push(ResolveError::UnknownDirective {
                                name: other.to_string(),
                                context: "action",
                                item_name: name.clone(),
                            });
                        }
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
                    let log = extract_log_spec(action_dirs, name, "action", &mut errors);
                    actions.push(ActionDef {
                        name: name.clone(),
                        param_types,
                        param_names: param_names.clone(),
                        return_type,
                        body: body.clone(),
                        tests,
                        log,
                    });
                }
            }
            _ => {}
        }
    }

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
        functions,
        actions,
    })
}

/// Extrai `TestSpec` das diretivas `@test` de uma `ActionDecl`.
///
/// Suporta duas formas:
/// - `@test("desc")` — tupla posicional: `desc` é o primeiro `Expr`.
/// - `@test{desc: "...", args: (1,2), timeout: 5000, expects: "Panic: msg"}`
///   — dict nomeado: chaves `desc`, `args`, `timeout`, `expects`.
///
/// Valida tipos dos valores: `desc`/`expects` devem ser `TextLit`,
/// `timeout` deve ser `IntLit`, `args` pode ser qualquer `Expr` (o inference tipa).
fn extract_test_specs(
    directives: &[kata_ast::Directive],
    action_name: &str,
    errors: &mut Vec<ResolveError>,
) -> Vec<TestSpec> {
    let mut specs = Vec::new();
    for d in directives {
        if d.name != "test" {
            continue;
        }
        let mut spec = TestSpec {
            desc: None,
            args: None,
            timeout: None,
            expects: None,
        };
        match d.args.as_slice() {
            // @test("desc") — forma curta: 1 posicional.
            [kata_ast::DirectiveArg::Expr(e)] => {
                if let kata_ast::Expr::TextLit { text } = &e.node {
                    spec.desc = Some(text.clone());
                } else {
                    errors.push(ResolveError::UnknownDirective {
                        name: "test".into(),
                        context: "action",
                        item_name: format!("{action_name}: desc posicional deve ser Text"),
                    });
                }
            }
            // @test{desc: "...", args: (...), timeout: N, expects: "..."}
            args if args
                .iter()
                .all(|a| matches!(a, kata_ast::DirectiveArg::Named { .. })) =>
            {
                for arg in args {
                    if let kata_ast::DirectiveArg::Named { key, value } = arg {
                        match key.as_str() {
                            "desc" => {
                                if let kata_ast::Expr::TextLit { text } = &value.node {
                                    spec.desc = Some(text.clone());
                                } else {
                                    errors.push(ResolveError::UnknownDirective {
                                        name: "test".into(),
                                        context: "action",
                                        item_name: format!("{action_name}: desc deve ser Text"),
                                    });
                                }
                            }
                            "args" => {
                                spec.args = Some((**value).clone());
                            }
                            "timeout" => {
                                if let kata_ast::Expr::IntLit { text } = &value.node {
                                    if let Ok(n) = text.parse::<i64>() {
                                        spec.timeout = Some(n);
                                    } else {
                                        errors.push(ResolveError::UnknownDirective {
                                            name: "test".into(),
                                            context: "action",
                                            item_name: format!(
                                                "{action_name}: timeout inválido: {text}"
                                            ),
                                        });
                                    }
                                } else {
                                    errors.push(ResolveError::UnknownDirective {
                                        name: "test".into(),
                                        context: "action",
                                        item_name: format!("{action_name}: timeout deve ser Int"),
                                    });
                                }
                            }
                            "expects" => {
                                if let kata_ast::Expr::TextLit { text } = &value.node {
                                    spec.expects = Some(text.clone());
                                } else {
                                    errors.push(ResolveError::UnknownDirective {
                                        name: "test".into(),
                                        context: "action",
                                        item_name: format!("{action_name}: expects deve ser Text"),
                                    });
                                }
                            }
                            other => {
                                errors.push(ResolveError::UnknownDirective {
                                    name: "test".into(),
                                    context: "action",
                                    item_name: format!(
                                        "{action_name}: chave desconhecida: {other}"
                                    ),
                                });
                            }
                        }
                    }
                }
            }
            // @test() — sem args, ou mistura inválida.
            [] => {}
            _ => {
                errors.push(ResolveError::UnknownDirective {
                    name: "test".into(),
                    context: "action",
                    item_name: format!(
                        "{action_name}: args de @test devem ser todos posicionais ou todos nomeados"
                    ),
                });
            }
        }
        specs.push(spec);
    }
    specs
}

/// Extrai `LogSpec` da diretiva `@log` se presente.
///
/// `@log{msg: "...", when: "enter"/"exit", topic: "...", policy: "...", level: "Info"}`
///
/// `msg` e `when` são obrigatórios. `topic`, `policy`, `level` são opcionais.
/// Retorna `None` se não há diretiva `@log`.
fn extract_log_spec(
    directives: &[kata_ast::Directive],
    item_name: &str,
    context: &'static str,
    errors: &mut Vec<ResolveError>,
) -> Option<LogSpec> {
    let log_dir = directives.iter().find(|d| d.name == "log")?;
    let mut msg = None;
    let mut when = None;
    let mut topic = None;
    let mut policy = None;
    let mut level = None;

    for arg in &log_dir.args {
        if let kata_ast::DirectiveArg::Named { key, value } = arg {
            match key.as_str() {
                "msg" => {
                    if let kata_ast::Expr::TextLit { text } = &value.node {
                        msg = Some(text.clone());
                    } else {
                        errors.push(ResolveError::UnknownDirective {
                            name: "log".into(),
                            context,
                            item_name: format!("{item_name}: msg deve ser Text"),
                        });
                    }
                }
                "when" => {
                    if let kata_ast::Expr::TextLit { text } = &value.node {
                        when = Some(text.clone());
                    } else {
                        errors.push(ResolveError::UnknownDirective {
                            name: "log".into(),
                            context,
                            item_name: format!("{item_name}: when deve ser Text"),
                        });
                    }
                }
                "topic" => {
                    if let kata_ast::Expr::TextLit { text } = &value.node {
                        topic = Some(text.clone());
                    } else {
                        errors.push(ResolveError::UnknownDirective {
                            name: "log".into(),
                            context,
                            item_name: format!("{item_name}: topic deve ser Text"),
                        });
                    }
                }
                "policy" => {
                    if let kata_ast::Expr::TextLit { text } = &value.node {
                        policy = Some(text.clone());
                    } else {
                        errors.push(ResolveError::UnknownDirective {
                            name: "log".into(),
                            context,
                            item_name: format!("{item_name}: policy deve ser Text"),
                        });
                    }
                }
                "level" => {
                    // Level pode ser TextLit ("Info") ou VariantQual (LogLevel::Info).
                    if let kata_ast::Expr::TextLit { text } = &value.node {
                        level = Some(text.clone());
                    } else if let kata_ast::Expr::VariantQual { variant, .. } = &value.node {
                        level = Some(variant.clone());
                    } else {
                        errors.push(ResolveError::UnknownDirective {
                            name: "log".into(),
                            context,
                            item_name: format!(
                                "{item_name}: level deve ser Text ou variante de LogLevel"
                            ),
                        });
                    }
                }
                other => {
                    errors.push(ResolveError::UnknownDirective {
                        name: "log".into(),
                        context,
                        item_name: format!("{item_name}: chave desconhecida: {other}"),
                    });
                }
            }
        }
    }

    // Valida campos obrigatórios.
    let msg = msg.unwrap_or_else(|| {
        errors.push(ResolveError::UnknownDirective {
            name: "log".into(),
            context,
            item_name: format!("{item_name}: msg é obrigatório em @log"),
        });
        String::new()
    });
    let when = when.unwrap_or_else(|| {
        errors.push(ResolveError::UnknownDirective {
            name: "log".into(),
            context,
            item_name: format!("{item_name}: when é obrigatório em @log"),
        });
        String::new()
    });

    // Valida valor de when.
    if when != "enter" && when != "exit" {
        errors.push(ResolveError::UnknownDirective {
            name: "log".into(),
            context,
            item_name: format!("{item_name}: when deve ser \"enter\" ou \"exit\", got \"{when}\""),
        });
    }

    Some(LogSpec {
        msg,
        when,
        topic,
        policy,
        level,
    })
}

pub use prelude_sigs::load_prelude;
