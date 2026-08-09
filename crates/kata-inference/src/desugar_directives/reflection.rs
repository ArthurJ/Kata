//! Informações de reflexão e síntese de variáveis de reflexão.

use kata_ast::{ActionStmt, Expr, Span, Spanned};
use kata_core::Ty;

// ── Informações de reflexão ─────────────────────────────────────────

/// Informações estáticas sobre o item decorado, usadas para sintetizar
/// as variáveis de reflexão.
pub(super) struct ReflectionInfo {
    pub(super) name: String,
    pub(super) arity: usize,
    /// Nomes dos params para `_args`. Usa o nome se `Some`, `__arg_{i}` se `None`.
    pub(super) arg_idents: Vec<String>,
    /// Strings de tipos para `_types`.
    pub(super) type_strings: Vec<String>,
    pub(super) return_type_string: String,
    pub(super) is_action: bool,
    /// Se false, não sintetiza `_args` (funções puras não têm param_names).
    pub(super) has_args: bool,
}

impl ReflectionInfo {
    pub(super) fn for_action(
        name: &str,
        param_types: &[Ty],
        param_names: &[Option<String>],
        ret: &Ty,
    ) -> Self {
        Self::new(name, param_types, param_names, ret, true, true)
    }

    pub(super) fn for_function(name: &str, param_types: &[Ty], ret: &Ty) -> Self {
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
pub(super) fn synthesize_static_bindings(refl: &ReflectionInfo) -> Vec<Spanned<Expr>> {
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
pub(super) fn synthesize_args_binding(refl: &ReflectionInfo) -> Spanned<Expr> {
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
pub(super) fn synthesize_return_binding() -> Spanned<Expr> {
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
pub(super) fn action_stmts_to_exprs(stmts: &[ActionStmt]) -> Vec<Spanned<Expr>> {
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
pub(super) fn directive_body_to_expr(stmts: &[ActionStmt]) -> Spanned<Expr> {
    let span = Span::synthetic();
    let exprs = action_stmts_to_exprs(stmts);
    if exprs.len() == 1 {
        return exprs
            .into_iter()
            .next()
            .expect("len()==1 garante next()=Some");
    }
    Spanned {
        node: Expr::Block { stmts: exprs },
        span,
    }
}
