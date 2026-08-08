//! Desugaring de diretivas customizadas — inlining de bodies.
//!
//! Passada separada entre `resolve` e `infer_module` no driver.
//! Transforma a AST (`Module`) expandindo diretivas customizadas (`@nome`)
//! aplicadas em `Item::ActionDecl` e `Item::Sig`, inlineando o body da diretiva
//! conforme o Hook (Enter, Exit, ShortCircuit, Transform).
//!
//! O desugaring produz AST expandida que o typeck valida normalmente.
//! As variáveis de reflexão (`_name`, `_arity`, `_types`, `_return_type`,
//! `_is_action`, `_args`, `_return`) são sintetizadas como `let` bindings.

use kata_ast::{ActionStmt, Expr, LambdaClause, MatchArm, Pattern, Span, Spanned};
use kata_core::Ty;
use kata_resolution::{DirectiveDef, DirectiveRegistry, Hook, ResolvedModule, Target};

/// Desugara diretivas customizadas em um `ResolvedModule`, aplicando inlining
/// nos bodies das actions e cláusulas das funções que têm diretivas customizadas.
///
/// Deve ser chamado entre `resolve` (que popula `DirectiveRegistry` e
/// `custom_directives` em ActionDef/FunctionDef) e `infer_module`.
pub fn desugar_directives(resolved: &mut ResolvedModule) {
    let registry = &resolved.directive_registry;

    // Actions: aplicar inlining nos bodies.
    for action in &mut resolved.actions {
        if action.custom_directives.is_empty() {
            continue;
        }
        let refl = ReflectionInfo::for_action(
            &action.name,
            &action.param_types,
            &action.param_names,
            &action.return_type,
        );
        action.body = apply_directives_to_action_body(
            std::mem::take(&mut action.body),
            &action.custom_directives,
            &refl,
            registry,
        );
    }

    // Functions: aplicar inlining nas cláusulas lambda.
    for func in &mut resolved.functions {
        if func.custom_directives.is_empty() {
            continue;
        }
        let refl = ReflectionInfo::for_function(&func.name, &func.param_types, &func.return_type);
        for clause in &mut func.clauses {
            clause.node = apply_directives_to_lambda_clause(
                clause.node.clone(),
                &func.custom_directives,
                &refl,
                registry,
            );
        }
    }
}

// ── Informações de reflexão ─────────────────────────────────────────

/// Informações estáticas sobre o item decorado, usadas para sintetizar
/// as variáveis de reflexão.
struct ReflectionInfo {
    name: String,
    arity: usize,
    /// Nomes dos params para `_args`. Usa o nome se `Some`, `__arg_{i}` se `None`.
    arg_idents: Vec<String>,
    /// Strings de tipos para `_types`.
    type_strings: Vec<String>,
    return_type_string: String,
    is_action: bool,
    /// Se false, não sintetiza `_args` (funções puras não têm param_names).
    has_args: bool,
}

impl ReflectionInfo {
    fn for_action(
        name: &str,
        param_types: &[Ty],
        param_names: &[Option<String>],
        ret: &Ty,
    ) -> Self {
        Self::new(name, param_types, param_names, ret, true, true)
    }

    fn for_function(name: &str, param_types: &[Ty], ret: &Ty) -> Self {
        let param_names: Vec<Option<String>> = (0..param_types.len()).map(|_| None).collect();
        Self::new(name, param_types, &param_names, ret, false, false)
    }

    fn new(
        name: &str,
        param_types: &[Ty],
        param_names: &[Option<String>],
        ret: &Ty,
        is_action: bool,
        has_args: bool,
    ) -> Self {
        let arg_idents = param_names
            .iter()
            .enumerate()
            .map(|(i, pn)| pn.clone().unwrap_or_else(|| format!("__arg_{i}")))
            .collect();
        let type_strings = param_types.iter().map(|t| t.to_string()).collect();
        ReflectionInfo {
            name: name.to_string(),
            arity: param_types.len(),
            arg_idents,
            type_strings,
            return_type_string: ret.to_string(),
            is_action,
            has_args,
        }
    }
}

// ── Síntese de variáveis de reflexão ─────────────────────────────────

/// Sintetiza `let` bindings das variáveis de reflexão estáticas como
/// expressões `Expr::Let`.
/// `_name`, `_arity`, `_types`, `_return_type`, `_is_action`.
fn synthesize_static_bindings(refl: &ReflectionInfo) -> Vec<Spanned<Expr>> {
    let span = Span::synthetic();
    vec![
        // let _name := "nome"
        Spanned {
            node: Expr::Let {
                name: "_name".into(),
                value: Box::new(Spanned {
                    node: Expr::TextLit {
                        text: refl.name.clone(),
                    },
                    span,
                }),
            },
            span,
        },
        // let _arity := N
        Spanned {
            node: Expr::Let {
                name: "_arity".into(),
                value: Box::new(Spanned {
                    node: Expr::IntLit {
                        text: refl.arity.to_string(),
                    },
                    span,
                }),
            },
            span,
        },
        // let _types := ["T1", "T2", ...]
        Spanned {
            node: Expr::Let {
                name: "_types".into(),
                value: Box::new(Spanned {
                    node: Expr::ListLit {
                        elements: refl
                            .type_strings
                            .iter()
                            .map(|s| Spanned {
                                node: Expr::TextLit { text: s.clone() },
                                span,
                            })
                            .collect(),
                    },
                    span,
                }),
            },
            span,
        },
        // let _return_type := "TRet"
        Spanned {
            node: Expr::Let {
                name: "_return_type".into(),
                value: Box::new(Spanned {
                    node: Expr::TextLit {
                        text: refl.return_type_string.clone(),
                    },
                    span,
                }),
            },
            span,
        },
        // let _is_action := True/False
        Spanned {
            node: Expr::Let {
                name: "_is_action".into(),
                value: Box::new(Spanned {
                    node: Expr::VariantQual {
                        enum_name: "Boolean".into(),
                        variant: if refl.is_action { "True" } else { "False" }.into(),
                        module_path: None,
                    },
                    span,
                }),
            },
            span,
        },
    ]
}

/// Sintetiza `let _args := (x, y, ...)` como `Expr::Tuple` dos params.
/// Se `has_args = false` (funções puras sem param_names), gera `let _args := ()`.
fn synthesize_args_binding(refl: &ReflectionInfo) -> Spanned<Expr> {
    let span = Span::synthetic();
    if !refl.has_args {
        return Spanned {
            node: Expr::Let {
                name: "_args".into(),
                value: Box::new(Spanned {
                    node: Expr::Unit,
                    span,
                }),
            },
            span,
        };
    }
    let elements: Vec<Spanned<Expr>> = refl
        .arg_idents
        .iter()
        .map(|name| Spanned {
            node: Expr::Ident { name: name.clone() },
            span,
        })
        .collect();
    Spanned {
        node: Expr::Let {
            name: "_args".into(),
            value: Box::new(Spanned {
                node: Expr::Tuple { elements },
                span,
            }),
        },
        span,
    }
}

/// Sintetiza `let _return := __result`.
fn synthesize_return_binding() -> Spanned<Expr> {
    let span = Span::synthetic();
    Spanned {
        node: Expr::Let {
            name: "_return".into(),
            value: Box::new(Spanned {
                node: Expr::Ident {
                    name: "__result".into(),
                },
                span,
            }),
        },
        span,
    }
}

/// Converte `ActionStmt` (body de diretiva) em `Spanned<Expr>`.
/// Cada `ActionStmt` vira um `Spanned<Expr>` preservando o span.
fn action_stmts_to_exprs(stmts: &[ActionStmt]) -> Vec<Spanned<Expr>> {
    stmts
        .iter()
        .map(|s| Spanned {
            node: s.expr.node.clone(),
            span: s.expr.span,
        })
        .collect()
}

/// Converte o body de uma diretiva (`Vec<ActionStmt>`) em uma única `Expr`.
/// Se há 1 statement, retorna a expr diretamente.
/// Se há N statements, envolve em `Expr::Block`.
fn directive_body_to_expr(stmts: &[ActionStmt]) -> Spanned<Expr> {
    let span = Span::synthetic();
    let exprs = action_stmts_to_exprs(stmts);
    if exprs.len() == 1 {
        return exprs.into_iter().next().unwrap();
    }
    Spanned {
        node: Expr::Block { stmts: exprs },
        span,
    }
}

// ── Aplicação de diretivas ───────────────────────────────────────────

/// Aplica diretivas customizadas ao body de uma Action.
/// `custom_names` em ordem: primeira = mais externa.
fn apply_directives_to_action_body(
    body: Vec<ActionStmt>,
    custom_names: &[String],
    refl: &ReflectionInfo,
    registry: &DirectiveRegistry,
) -> Vec<ActionStmt> {
    // Processar de dentro para fora: última diretiva é a mais interna.
    // Aplicamos de trás para frente para que a primeira diretiva envolva todas.
    let mut current_body = body;

    for name in custom_names.iter().rev() {
        // Coletar todas as defs aplicáveis a este item (action).
        let defs: Vec<&DirectiveDef> = registry
            .lookup_by_name(name)
            .into_iter()
            .filter(|d| matches!(d.key.on, Target::Action | Target::Any))
            .collect();

        if defs.is_empty() {
            continue;
        }

        // Para cada hook presente, aplicar o inlining correspondente.
        // Múltiplos hooks da mesma diretiva = múltiplas injeções.
        for def in &defs {
            current_body = apply_hook_to_action_body(current_body, def, refl);
        }
    }

    current_body
}

/// Aplica um hook específico ao body de uma Action.
fn apply_hook_to_action_body(
    body: Vec<ActionStmt>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Vec<ActionStmt> {
    match def.key.when {
        Hook::Enter => apply_enter_to_action_body(body, def, refl),
        Hook::Exit => apply_exit_to_action_body(body, def, refl),
        Hook::ShortCircuit => apply_shortcircuit_to_action_body(body, def, refl),
        Hook::Transform => apply_transform_to_action_body(body, def, refl),
    }
}

/// Enter: prependa bindings de reflexão + statements da diretiva antes do body.
fn apply_enter_to_action_body(
    body: Vec<ActionStmt>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Vec<ActionStmt> {
    let span = Span::synthetic();

    // Bindings estáticos
    let mut injected: Vec<ActionStmt> = synthesize_static_bindings(refl)
        .into_iter()
        .map(|e| ActionStmt {
            expr: e,
            has_semicolon: true,
        })
        .collect();

    // _args binding
    injected.push(ActionStmt {
        expr: synthesize_args_binding(refl),
        has_semicolon: true,
    });

    // Statements do body da diretiva
    for stmt in &def.body {
        injected.push(ActionStmt {
            expr: Spanned {
                node: stmt.expr.node.clone(),
                span: stmt.expr.span,
            },
            has_semicolon: stmt.has_semicolon,
        });
    }

    // Body original
    injected.extend(body);
    let _ = span;
    injected
}

/// Exit: envolve todos os pontos de saída com `let __result := ...; <bindings>;
/// <body da diretiva>; __result`.
fn apply_exit_to_action_body(
    body: Vec<ActionStmt>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Vec<ActionStmt> {
    if body.is_empty() {
        return body;
    }

    let total = body.len();
    let mut result = Vec::with_capacity(total + 16);

    for (i, stmt) in body.into_iter().enumerate() {
        let is_last = i == total - 1;
        if is_last && !stmt.has_semicolon {
            // Retorno implícito — envolver a expr.
            let wrapped = wrap_exit_expr(&stmt.expr, def, refl);
            result.push(ActionStmt {
                expr: wrapped,
                has_semicolon: false,
            });
        } else if let Expr::Return(inner) = &stmt.expr.node {
            // return expr — envolver.
            let wrapped = wrap_exit_expr(inner, def, refl);
            result.push(ActionStmt {
                expr: Spanned {
                    node: Expr::Return(Box::new(wrapped)),
                    span: stmt.expr.span,
                },
                has_semicolon: stmt.has_semicolon,
            });
        } else {
            // Statement intermediário — pode conter return em sub-expressões.
            let transformed = transform_returns_in_expr(stmt.expr.node.clone(), &|e| {
                wrap_exit_expr(e, def, refl)
            });
            result.push(ActionStmt {
                expr: Spanned {
                    node: transformed,
                    span: stmt.expr.span,
                },
                has_semicolon: stmt.has_semicolon,
            });
        }
    }

    result
}

/// Envolve uma expressão de saída com `let __result := <expr>; <bindings>;
/// <body da diretiva>; __result` como um `Expr::Block`.
fn wrap_exit_expr(
    expr: &Spanned<Expr>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Spanned<Expr> {
    let span = Span::synthetic();

    let mut stmts = Vec::new();

    // let __result := <expr>
    stmts.push(Spanned {
        node: Expr::Let {
            name: "__result".into(),
            value: Box::new(Spanned {
                node: expr.node.clone(),
                span: expr.span,
            }),
        },
        span,
    });

    // Bindings estáticos
    stmts.extend(synthesize_static_bindings(refl));

    // _args binding
    stmts.push(synthesize_args_binding(refl));

    // _return binding
    stmts.push(synthesize_return_binding());

    // Statements do body da diretiva
    stmts.extend(action_stmts_to_exprs(&def.body));

    // __result como valor de retorno
    stmts.push(Spanned {
        node: Expr::Ident {
            name: "__result".into(),
        },
        span,
    });

    Spanned {
        node: Expr::Block { stmts },
        span,
    }
}

/// ShortCircuit: insere `let __decision := <body da diretiva>; match __decision
/// { Optional::Some(r): r, Optional::None: <body original> }` no início.
fn apply_shortcircuit_to_action_body(
    body: Vec<ActionStmt>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Vec<ActionStmt> {
    let span = Span::synthetic();

    let mut injected: Vec<ActionStmt> = Vec::new();

    // Bindings estáticos
    for e in synthesize_static_bindings(refl) {
        injected.push(ActionStmt {
            expr: e,
            has_semicolon: true,
        });
    }

    // _args binding
    injected.push(ActionStmt {
        expr: synthesize_args_binding(refl),
        has_semicolon: true,
    });

    // let __decision := <body da diretiva>
    let decision_expr = directive_body_to_expr(&def.body);
    injected.push(ActionStmt {
        expr: Spanned {
            node: Expr::Let {
                name: "__decision".into(),
                value: Box::new(decision_expr),
            },
            span,
        },
        has_semicolon: true,
    });

    // match __decision { Optional::Some(r): r, Optional::None: <body original> }
    let body_exprs: Vec<Spanned<Expr>> = body
        .into_iter()
        .map(|s| Spanned {
            node: s.expr.node,
            span: s.expr.span,
        })
        .collect();

    let original_body = if body_exprs.len() == 1 {
        body_exprs.into_iter().next().unwrap()
    } else {
        Spanned {
            node: Expr::Block { stmts: body_exprs },
            span,
        }
    };

    let match_expr = Spanned {
        node: Expr::Match {
            scrutinee: Box::new(Spanned {
                node: Expr::Ident {
                    name: "__decision".into(),
                },
                span,
            }),
            arms: vec![
                MatchArm {
                    pattern: Some(Spanned {
                        node: Pattern::Variant {
                            enum_name: "Optional".into(),
                            variant: "Some".into(),
                            payload: Some(vec![Spanned {
                                node: Pattern::Ident("r".into()),
                                span,
                            }]),
                        },
                        span,
                    }),
                    guard: None,
                    body: Spanned {
                        node: Expr::Ident { name: "r".into() },
                        span,
                    },
                },
                MatchArm {
                    pattern: Some(Spanned {
                        node: Pattern::Variant {
                            enum_name: "Optional".into(),
                            variant: "None".into(),
                            payload: None,
                        },
                        span,
                    }),
                    guard: None,
                    body: original_body,
                },
            ],
        },
        span,
    };

    injected.push(ActionStmt {
        expr: match_expr,
        has_semicolon: false,
    });

    injected
}

/// Transform: envolve pontos de saída com `let __result := ...; <bindings>;
/// <body da diretiva (último statement = valor transformado)>`.
fn apply_transform_to_action_body(
    body: Vec<ActionStmt>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Vec<ActionStmt> {
    if body.is_empty() {
        return body;
    }

    let total = body.len();
    let mut result = Vec::with_capacity(total + 16);

    for (i, stmt) in body.into_iter().enumerate() {
        let is_last = i == total - 1;
        if is_last && !stmt.has_semicolon {
            // Retorno implícito — envolver a expr.
            let wrapped = wrap_transform_expr(&stmt.expr, def, refl);
            result.push(ActionStmt {
                expr: wrapped,
                has_semicolon: false,
            });
        } else if let Expr::Return(inner) = &stmt.expr.node {
            // return expr — envolver.
            let wrapped = wrap_transform_expr(inner, def, refl);
            result.push(ActionStmt {
                expr: Spanned {
                    node: Expr::Return(Box::new(wrapped)),
                    span: stmt.expr.span,
                },
                has_semicolon: stmt.has_semicolon,
            });
        } else {
            // Statement intermediário — pode conter return em sub-expressões.
            let transformed = transform_returns_in_expr(stmt.expr.node.clone(), &|e| {
                wrap_transform_expr(e, def, refl)
            });
            result.push(ActionStmt {
                expr: Spanned {
                    node: transformed,
                    span: stmt.expr.span,
                },
                has_semicolon: stmt.has_semicolon,
            });
        }
    }

    result
}

/// Envolve uma expressão de saída com `let __result := <expr>; <bindings>;
/// <body da diretiva — último statement = valor transformado>` como um `Expr::Block`.
fn wrap_transform_expr(
    expr: &Spanned<Expr>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Spanned<Expr> {
    let span = Span::synthetic();

    let mut stmts = Vec::new();

    // let __result := <expr>
    stmts.push(Spanned {
        node: Expr::Let {
            name: "__result".into(),
            value: Box::new(Spanned {
                node: expr.node.clone(),
                span: expr.span,
            }),
        },
        span,
    });

    // Bindings estáticos
    stmts.extend(synthesize_static_bindings(refl));

    // _args binding
    stmts.push(synthesize_args_binding(refl));

    // _return binding
    stmts.push(synthesize_return_binding());

    // Statements do body da diretiva — o último é o valor transformado.
    stmts.extend(action_stmts_to_exprs(&def.body));

    Spanned {
        node: Expr::Block { stmts },
        span,
    }
}

// ── Transformação recursiva de returns ──────────────────────────────

/// Percorre uma `Expr` recursivamente, transformando todos os `Expr::Return`
/// encontrados. O closure recebe a expr interna do return e retorna a
/// expr envolvida.
fn transform_returns_in_expr<F>(expr: Expr, wrap: &F) -> Expr
where
    F: Fn(&Spanned<Expr>) -> Spanned<Expr>,
{
    match expr {
        Expr::Return(inner) => {
            let wrapped = wrap(&inner);
            Expr::Return(Box::new(wrapped))
        }
        Expr::Let { name, value } => Expr::Let {
            name,
            value: Box::new(transform_spanned_expr(*value, wrap)),
        },
        Expr::Var { name, value } => Expr::Var {
            name,
            value: Box::new(transform_spanned_expr(*value, wrap)),
        },
        Expr::Reassign { name, value } => Expr::Reassign {
            name,
            value: Box::new(transform_spanned_expr(*value, wrap)),
        },
        Expr::Apply { callee, args } => Expr::Apply {
            callee: Box::new(transform_spanned_expr(*callee, wrap)),
            args: args
                .into_iter()
                .map(|a| transform_spanned_expr(a, wrap))
                .collect(),
        },
        Expr::Match { scrutinee, arms } => Expr::Match {
            scrutinee: Box::new(transform_spanned_expr(*scrutinee, wrap)),
            arms: arms
                .into_iter()
                .map(|arm| MatchArm {
                    pattern: arm.pattern,
                    guard: arm.guard.map(|g| transform_spanned_expr(g, wrap)),
                    body: transform_spanned_expr(arm.body, wrap),
                })
                .collect(),
        },
        Expr::Block { stmts } => Expr::Block {
            stmts: stmts
                .into_iter()
                .map(|s| transform_spanned_expr(s, wrap))
                .collect(),
        },
        Expr::Loop { body } => Expr::Loop {
            body: body
                .into_iter()
                .map(|s| transform_spanned_expr(s, wrap))
                .collect(),
        },
        Expr::ForIn {
            var_name,
            iterable,
            body,
        } => Expr::ForIn {
            var_name,
            iterable: Box::new(transform_spanned_expr(*iterable, wrap)),
            body: body
                .into_iter()
                .map(|s| transform_spanned_expr(s, wrap))
                .collect(),
        },
        // Nós que não contêm sub-expressões com return potencial.
        other => other,
    }
}

fn transform_spanned_expr<F>(expr: Spanned<Expr>, wrap: &F) -> Spanned<Expr>
where
    F: Fn(&Spanned<Expr>) -> Spanned<Expr>,
{
    Spanned {
        node: transform_returns_in_expr(expr.node, wrap),
        span: expr.span,
    }
}

// ── Aplicação em LambdaClause (funções puras) ───────────────────────

/// Aplica diretivas customizadas a uma `LambdaClause` de `Item::Sig`.
fn apply_directives_to_lambda_clause(
    clause: LambdaClause,
    custom_names: &[String],
    refl: &ReflectionInfo,
    registry: &DirectiveRegistry,
) -> LambdaClause {
    let mut current_body = clause.body;

    // Processar de dentro para fora.
    for name in custom_names.iter().rev() {
        // Coletar defs aplicáveis a este item (função).
        let defs: Vec<&DirectiveDef> = registry
            .lookup_by_name(name)
            .into_iter()
            .filter(|d| matches!(d.key.on, Target::Function | Target::Any))
            .collect();

        if defs.is_empty() {
            continue;
        }

        for def in &defs {
            current_body = apply_hook_to_lambda_body(current_body, def, refl);
        }
    }

    LambdaClause {
        patterns: clause.patterns,
        body: current_body,
        guards: clause.guards,
        with_bindings: clause.with_bindings,
    }
}

/// Aplica um hook específico ao body de uma função pura (uma `Spanned<Expr>`).
fn apply_hook_to_lambda_body(
    body: Spanned<Expr>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Spanned<Expr> {
    match def.key.when {
        Hook::Enter => apply_enter_to_lambda_body(body, def, refl),
        Hook::Exit => apply_exit_to_lambda_body(body, def, refl),
        Hook::ShortCircuit | Hook::Transform => {
            // ShortCircuit e Transform não podem decorar funções — o resolution
            // já rejeitou a combinação. Mas se chegamos aqui, o body da diretiva
            // tem Target::Any ou Target::Function com ShortCircuit/Transform, o que
            // é impossível. Retornar inalterado.
            body
        }
    }
}

/// Enter em função pura: prependa bindings + statements da diretiva
/// antes do body, envolvendo em `Expr::Block`.
fn apply_enter_to_lambda_body(
    body: Spanned<Expr>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Spanned<Expr> {
    let span = Span::synthetic();
    let mut stmts = Vec::new();

    // Bindings estáticos
    stmts.extend(synthesize_static_bindings(refl));

    // _args binding
    stmts.push(synthesize_args_binding(refl));

    // Statements do body da diretiva
    stmts.extend(action_stmts_to_exprs(&def.body));

    // Body original
    stmts.push(body);

    Spanned {
        node: Expr::Block { stmts },
        span,
    }
}

/// Exit em função pura: envolve o body com `let __result := ...; <bindings>;
/// <body da diretiva>; __result` em `Expr::Block`.
fn apply_exit_to_lambda_body(
    body: Spanned<Expr>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Spanned<Expr> {
    let span = Span::synthetic();
    let mut stmts = Vec::new();

    // let __result := <body>
    stmts.push(Spanned {
        node: Expr::Let {
            name: "__result".into(),
            value: Box::new(body),
        },
        span,
    });

    // Bindings estáticos
    stmts.extend(synthesize_static_bindings(refl));

    // _args binding
    stmts.push(synthesize_args_binding(refl));

    // _return binding
    stmts.push(synthesize_return_binding());

    // Statements do body da diretiva
    stmts.extend(action_stmts_to_exprs(&def.body));

    // __result como valor de retorno
    stmts.push(Spanned {
        node: Expr::Ident {
            name: "__result".into(),
        },
        span,
    });

    Spanned {
        node: Expr::Block { stmts },
        span,
    }
}
