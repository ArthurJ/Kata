//! Parsers de diretivas `@test` e `@log`.
//!
//! Extrai `TestSpec` e `LogSpec` das diretivas AST, validando tipos dos
//! argumentos e coletando erros em `Vec<ResolveError>`.

use kata_ast::{Directive, DirectiveArg, Expr};

use super::types::{DirectiveDef, DirectiveKey, Hook, LogSpec, ResolveError, Target, TestSpec, TimerSpec};

/// Extrai `TestSpec` das diretivas `@test` de uma `ActionDecl`.
///
/// Suporta duas formas:
/// - `@test("desc")` — tupla posicional: `desc` é o primeiro `Expr`.
/// - `@test{desc: \"...\", args: (1,2), timeout: 5000, expects: \"Panic: msg\"}`
///   — dict nomeado: chaves `desc`, `args`, `timeout`, `expects`.
///
/// Valida tipos dos valores: `desc`/`expects` devem ser `TextLit`,
/// `timeout` deve ser `IntLit`, `args` pode ser qualquer `Expr` (o inference tipa).
pub(crate) fn extract_test_specs(
    directives: &[Directive],
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
            [DirectiveArg::Expr(e)] => {
                if let Expr::TextLit { text } = &e.node {
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
            args if args.iter().all(|a| matches!(a, DirectiveArg::Named { .. })) => {
                for arg in args {
                    if let DirectiveArg::Named { key, value } = arg {
                        match key.as_str() {
                            "desc" => {
                                if let Expr::TextLit { text } = &value.node {
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
                                if let Expr::IntLit { text } = &value.node {
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
                                if let Expr::TextLit { text } = &value.node {
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
/// `@log{msg: \"...\", when: \"enter\"/\"exit\", topic: \"...\", policy: \"...\", level: \"Info\"}`
///
/// `msg` e `when` são obrigatórios. `topic`, `policy`, `level` são opcionais.
/// Retorna `None` se não há diretiva `@log`.
pub(crate) fn extract_log_spec(
    directives: &[Directive],
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
        if let DirectiveArg::Named { key, value } = arg {
            match key.as_str() {
                "msg" => {
                    if let Expr::TextLit { text } = &value.node {
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
                    if let Expr::TextLit { text } = &value.node {
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
                    if let Expr::TextLit { text } = &value.node {
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
                    if let Expr::TextLit { text } = &value.node {
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
                    if let Expr::TextLit { text } = &value.node {
                        level = Some(text.clone());
                    } else if let Expr::VariantQual { variant, .. } = &value.node {
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

/// Extrai `TimerSpec` da diretiva `@timer` se presente.
///
/// `@timer{topic: "...", stats: true/false, repeat: N, msg: "..."}`
///
/// Todos os argumentos são opcionais. Retorna `None` se não há diretiva `@timer`.
pub(crate) fn extract_timer_spec(
    directives: &[Directive],
    item_name: &str,
    context: &'static str,
    errors: &mut Vec<ResolveError>,
) -> Option<TimerSpec> {
    let timer_dir = directives.iter().find(|d| d.name == "timer")?;
    let mut topic = None;
    let mut stats = None;
    let mut repeat = None;
    let mut msg = None;

    for arg in &timer_dir.args {
        if let DirectiveArg::Named { key, value } = arg {
            match key.as_str() {
                "topic" => {
                    if let Expr::TextLit { text } = &value.node {
                        topic = Some(text.clone());
                    } else {
                        errors.push(ResolveError::UnknownDirective {
                            name: "timer".into(),
                            context,
                            item_name: format!("{item_name}: topic deve ser Text"),
                        });
                    }
                }
                "stats" => {
                    if let Expr::IntLit { text } = &value.node {
                        // Bool true/false parseado como IntLit 1/0 em Kata.
                        stats = Some(text == "1");
                    } else if let Expr::TextLit { text } = &value.node {
                        stats = Some(text == "true");
                    } else {
                        errors.push(ResolveError::UnknownDirective {
                            name: "timer".into(),
                            context,
                            item_name: format!("{item_name}: stats deve ser Bool"),
                        });
                    }
                }
                "repeat" => {
                    if let Expr::IntLit { text } = &value.node {
                        if let Ok(n) = text.parse::<u32>() {
                            repeat = Some(n);
                        } else {
                            errors.push(ResolveError::UnknownDirective {
                                name: "timer".into(),
                                context,
                                item_name: format!("{item_name}: repeat inválido: {text}"),
                            });
                        }
                    } else {
                        errors.push(ResolveError::UnknownDirective {
                            name: "timer".into(),
                            context,
                            item_name: format!("{item_name}: repeat deve ser Int"),
                        });
                    }
                }
                "msg" => {
                    if let Expr::TextLit { text } = &value.node {
                        msg = Some(text.clone());
                    } else {
                        errors.push(ResolveError::UnknownDirective {
                            name: "timer".into(),
                            context,
                            item_name: format!("{item_name}: msg deve ser Text"),
                        });
                    }
                }
                other => {
                    errors.push(ResolveError::UnknownDirective {
                        name: "timer".into(),
                        context,
                        item_name: format!("{item_name}: chave desconhecida: {other}"),
                    });
                }
            }
        }
    }

    let stats = stats.unwrap_or(false);
    let repeat = repeat.unwrap_or(if stats { 10 } else { 1 });

    Some(TimerSpec {
        topic,
        stats,
        repeat,
        msg,
    })
}

/// Extrai `DirectiveDef` dos args de um `Item::DirectiveDecl`.
///
/// Args esperados: `{when: Hook::Enter, on: Target::Action}`.
/// `when` e `on` são obrigatórios e devem ser `Expr::VariantQual`
/// referenciando `enum Hook` e `enum Target` do prelude.
/// Retorna `Err` se os args são inválidos.
pub(crate) fn extract_directive_spec(
    name: &str,
    args: &[kata_ast::DirectiveArg],
    body: Vec<kata_ast::ActionStmt>,
) -> Result<DirectiveDef, ResolveError> {
    let mut when: Option<Hook> = None;
    let mut on: Option<Target> = None;

    for arg in args {
        let DirectiveArg::Named { key, value } = arg else {
            return Err(ResolveError::UnknownDirective {
                name: name.into(),
                context: "directive",
                item_name: "args devem ser nomeados".into(),
            });
        };
        match key.as_str() {
            "when" => {
                if let Expr::VariantQual {
                    enum_name, variant, ..
                } = &value.node
                {
                    when = parse_hook(enum_name, variant);
                }
                if when.is_none() {
                    return Err(ResolveError::UnknownDirective {
                        name: name.into(),
                        context: "directive",
                        item_name: "when deve ser Hook::Enter|Exit|ShortCircuit|Transform".into(),
                    });
                }
            }
            "on" => {
                if let Expr::VariantQual {
                    enum_name, variant, ..
                } = &value.node
                {
                    on = parse_target(enum_name, variant);
                }
                if on.is_none() {
                    return Err(ResolveError::UnknownDirective {
                        name: name.into(),
                        context: "directive",
                        item_name: "on deve ser Target::Action|Function|Any".into(),
                    });
                }
            }
            other => {
                return Err(ResolveError::UnknownDirective {
                    name: name.into(),
                    context: "directive",
                    item_name: format!("chave desconhecida: {other}"),
                });
            }
        }
    }

    let when = when.ok_or_else(|| ResolveError::UnknownDirective {
        name: name.into(),
        context: "directive",
        item_name: "when é obrigatório".into(),
    })?;
    let on = on.ok_or_else(|| ResolveError::UnknownDirective {
        name: name.into(),
        context: "directive",
        item_name: "on é obrigatório".into(),
    })?;

    // Validação estrutural: ShortCircuit e Transform exigem Target::Action.
    if matches!(when, Hook::ShortCircuit | Hook::Transform) && !matches!(on, Target::Action) {
        return Err(ResolveError::UnknownDirective {
            name: name.into(),
            context: "directive",
            item_name: format!("{when:?} exige Target::Action, got {on:?}"),
        });
    }

    Ok(DirectiveDef {
        key: DirectiveKey {
            name: name.into(),
            when,
            on,
        },
        body,
    })
}

fn parse_hook(enum_name: &str, variant: &str) -> Option<Hook> {
    if enum_name != "Hook" {
        return None;
    }
    match variant {
        "Enter" => Some(Hook::Enter),
        "Exit" => Some(Hook::Exit),
        "ShortCircuit" => Some(Hook::ShortCircuit),
        "Transform" => Some(Hook::Transform),
        _ => None,
    }
}

fn parse_target(enum_name: &str, variant: &str) -> Option<Target> {
    if enum_name != "Target" {
        return None;
    }
    match variant {
        "Action" => Some(Target::Action),
        "Function" => Some(Target::Function),
        "Any" => Some(Target::Any),
        _ => None,
    }
}
