//! `resolve_embeds` — resolve `@embed_text`/`@embed_bytes` substituindo
//! os nós efêmeros por literais (`TextLit`/`BytesLit`).
//!
//! Chamada entre parse e resolution. Vive em `kata-resolution` (a crate
//! do "mundo externo" — I/O de módulos, types de erro estruturados).
//!
//! O walker percorre `Module` → `Item` → `Expr`, substituindo nós
//! `EmbedText`/`EmbedBytes` por `TextLit`/`BytesLit`. Match exaustivo
//! em todas as variantes de `Expr` e `Item` — zero wildcard. Quando
//! uma nova variante for adicionada no futuro, o compilador Rust emite
//! E0004 (non-exhaustive match).
//!
//! Gabarito estrutural: `kata-inference/src/desugar.rs`.

use std::path::{Path, PathBuf};

use kata_ast::{
    ActionStmt, DirectiveArg, DotIndex, Expr, GuardClause, Item, LambdaClause, MatchArm, Module,
    Pattern, ReadMode, SelectArm, Spanned, WithBinding,
};
use kata_diagnostics::MietteSpan;
use thiserror::Error;

/// Erro durante resolução de `@embed_text`/`@embed_bytes`.
#[derive(Debug, Clone, Error, miette::Diagnostic)]
pub enum EmbedError {
    #[error("cannot embed file \"{path}\": {message}")]
    #[diagnostic(code = "resolve.embed_failed")]
    EmbedFailed {
        path: String,
        message: String,
        #[label("embed falhou")]
        span: MietteSpan,
    },

    #[error("embed not supported in embedded module (stdlib)")]
    #[diagnostic(code = "resolve.embed_in_stdlib")]
    EmbeddedModule {
        #[label("embed em módulo embedded")]
        span: MietteSpan,
    },
}

/// Resolve todos os `@embed_text`/`@embed_bytes` em um `Module`.
///
/// Lê arquivos do filesystem (relativo a `module_dir`) e substitui os
/// nós efêmeros por `TextLit`/`BytesLit`. Retorna o `Module` modificado
/// e a lista de arquivos embutidos (para rastreamento de dependências).
///
/// `module_dir` é o diretório do arquivo-fonte (para resolver paths
/// relativos). Path absoluto é aceito mas produz warning.
pub fn resolve_embeds(
    module: Module,
    module_dir: &Path,
) -> Result<(Module, Vec<PathBuf>), Vec<EmbedError>> {
    let mut ctx = EmbedCtx {
        module_dir,
        deps: Vec::new(),
        errors: Vec::new(),
        is_stdlib: false,
    };

    let mut items = Vec::with_capacity(module.items.len());
    for item in module.items {
        items.push(walk_item(item, &mut ctx));
    }

    if ctx.errors.is_empty() {
        Ok((Module { items }, ctx.deps))
    } else {
        Err(ctx.errors)
    }
}

/// Variante para stdlib: rejeita embeds com erro explícito.
pub fn resolve_embeds_stdlib(module: Module) -> Result<(Module, Vec<PathBuf>), Vec<EmbedError>> {
    let mut ctx = EmbedCtx {
        module_dir: Path::new("."),
        deps: Vec::new(),
        errors: Vec::new(),
        is_stdlib: true,
    };

    let mut items = Vec::with_capacity(module.items.len());
    for item in module.items {
        items.push(walk_item(item, &mut ctx));
    }

    if ctx.errors.is_empty() {
        Ok((Module { items }, ctx.deps))
    } else {
        Err(ctx.errors)
    }
}

struct EmbedCtx<'a> {
    module_dir: &'a Path,
    deps: Vec<PathBuf>,
    errors: Vec<EmbedError>,
    is_stdlib: bool,
}

// ── Walker: Item ──────────────────────────────────────────────────

fn walk_item(item: Spanned<Item>, ctx: &mut EmbedCtx) -> Spanned<Item> {
    let node = match item.node {
        Item::Sig {
            name,
            params,
            ret,
            directives,
            body,
        } => Item::Sig {
            name,
            params,
            ret,
            directives,
            body: body.map(|clauses| clauses.into_iter().map(|c| walk_clause(c, ctx)).collect()),
        },

        Item::ActionDecl {
            name,
            params,
            param_names,
            param_defaults,
            ret,
            directives,
            body,
        } => {
            let param_defaults = param_defaults
                .into_iter()
                .map(|d| d.map(|e| walk_expr(e, ctx)))
                .collect();
            let body = body
                .into_iter()
                .map(|stmt| walk_action_stmt(stmt, ctx))
                .collect();
            Item::ActionDecl {
                name,
                params,
                param_names,
                param_defaults,
                ret,
                directives,
                body,
            }
        }

        Item::ConstantDecl { name, value } => Item::ConstantDecl {
            name,
            value: walk_expr(value, ctx),
        },

        Item::EntryExpr(expr) => Item::EntryExpr(walk_expr(expr, ctx)),

        Item::DataDecl {
            name,
            fields,
            directives,
            refined,
        } => {
            let refined = refined.map(|r| kata_ast::RefinedDecl {
                base_ty: r.base_ty,
                predicates: r
                    .predicates
                    .into_iter()
                    .map(|p| walk_expr(p, ctx))
                    .collect(),
            });
            Item::DataDecl {
                name,
                fields,
                directives,
                refined,
            }
        }

        Item::EnumDecl {
            name,
            variants,
            directives,
        } => {
            let variants = variants
                .into_iter()
                .map(|v| kata_ast::VariantDecl {
                    name: v.name,
                    payload: v.payload,
                    default: v.default,
                    predicate: v.predicate.map(|p| walk_expr(p, ctx)),
                    fixed_value: v.fixed_value.map(|p| walk_expr(p, ctx)),
                })
                .collect();
            Item::EnumDecl {
                name,
                variants,
                directives,
            }
        }

        Item::InterfaceDecl {
            name,
            supertraits,
            type_params,
            signatures,
        } => {
            let signatures = signatures
                .into_iter()
                .map(|sig| kata_ast::InterfaceSig {
                    name: sig.name,
                    params: sig.params,
                    ret: sig.ret,
                    default_body: sig
                        .default_body
                        .map(|clauses| clauses.into_iter().map(|c| walk_clause(c, ctx)).collect()),
                })
                .collect();
            Item::InterfaceDecl {
                name,
                supertraits,
                type_params,
                signatures,
            }
        }

        Item::ImplementsDecl {
            type_name,
            type_params,
            interface_name,
            iface_params,
            methods,
        } => {
            let methods = methods
                .into_iter()
                .map(|m| kata_ast::ImplMethod {
                    name: m.name,
                    params: m.params,
                    ret: m.ret,
                    directives: m.directives,
                    body: m
                        .body
                        .map(|clauses| clauses.into_iter().map(|c| walk_clause(c, ctx)).collect()),
                })
                .collect();
            Item::ImplementsDecl {
                type_name,
                type_params,
                interface_name,
                iface_params,
                methods,
            }
        }

        Item::RefinesDecl {
            type_name,
            interface_name,
            methods,
        } => {
            let methods = methods
                .into_iter()
                .map(|m| kata_ast::ImplMethod {
                    name: m.name,
                    params: m.params,
                    ret: m.ret,
                    directives: m.directives,
                    body: m
                        .body
                        .map(|clauses| clauses.into_iter().map(|c| walk_clause(c, ctx)).collect()),
                })
                .collect();
            Item::RefinesDecl {
                type_name,
                interface_name,
                methods,
            }
        }

        Item::DirectiveDecl { name, args, body } => {
            let args = args
                .into_iter()
                .map(|a| match a {
                    DirectiveArg::Expr(e) => DirectiveArg::Expr(Box::new(walk_expr(*e, ctx))),
                    DirectiveArg::Named { key, value } => DirectiveArg::Named {
                        key,
                        value: Box::new(walk_expr(*value, ctx)),
                    },
                })
                .collect();
            let body = body
                .into_iter()
                .map(|stmt| walk_action_stmt(stmt, ctx))
                .collect();
            Item::DirectiveDecl { name, args, body }
        }

        // ImportDecl, ExportDecl, AliasDecl — sem Expr, retornam self.
        item @ (Item::ImportDecl { .. } | Item::ExportDecl { .. } | Item::AliasDecl { .. }) => item,
    };

    Spanned::new(node, item.span)
}

// ── Walker: LambdaClause, GuardClause, MatchArm, ActionStmt ──────

fn walk_clause(clause: Spanned<LambdaClause>, ctx: &mut EmbedCtx) -> Spanned<LambdaClause> {
    let node = LambdaClause {
        patterns: clause.node.patterns,
        body: walk_expr(clause.node.body, ctx),
        synthetic_pre: clause
            .node
            .synthetic_pre
            .into_iter()
            .map(|e| walk_expr(e, ctx))
            .collect(),
        synthetic_post: clause
            .node
            .synthetic_post
            .into_iter()
            .map(|e| walk_expr(e, ctx))
            .collect(),
        guards: clause
            .node
            .guards
            .into_iter()
            .map(|g| walk_guard(g, ctx))
            .collect(),
        with_bindings: clause
            .node
            .with_bindings
            .into_iter()
            .map(|w| walk_with_binding(w, ctx))
            .collect(),
    };
    Spanned::new(node, clause.span)
}

fn walk_guard(guard: GuardClause, ctx: &mut EmbedCtx) -> GuardClause {
    GuardClause {
        condition: guard.condition.map(|c| walk_expr(c, ctx)),
        body: walk_expr(guard.body, ctx),
    }
}

fn walk_with_binding(wb: WithBinding, ctx: &mut EmbedCtx) -> WithBinding {
    WithBinding {
        name: wb.name,
        value: walk_expr(wb.value, ctx),
    }
}

fn walk_action_stmt(stmt: ActionStmt, ctx: &mut EmbedCtx) -> ActionStmt {
    ActionStmt {
        expr: walk_expr(stmt.expr, ctx),
        has_semicolon: stmt.has_semicolon,
    }
}

// ── Walker: Expr (match exaustivo, zero wildcard) ─────────────────

fn walk_expr(expr: Spanned<Expr>, ctx: &mut EmbedCtx) -> Spanned<Expr> {
    let span = expr.span;
    let node = match expr.node {
        // ── Embed: substituir por literal ──
        Expr::EmbedText { path } => {
            if ctx.is_stdlib {
                ctx.errors.push(EmbedError::EmbeddedModule {
                    span: MietteSpan::from(span),
                });
                return Spanned::new(
                    Expr::TextLit {
                        text: String::new(),
                    },
                    span,
                );
            }
            match read_file(&path, ctx.module_dir, &span, ctx) {
                Ok(content) => Expr::TextLit { text: content },
                Err(e) => {
                    ctx.errors.push(e);
                    Expr::TextLit {
                        text: String::new(),
                    }
                }
            }
        }
        Expr::EmbedBytes { path } => {
            if ctx.is_stdlib {
                ctx.errors.push(EmbedError::EmbeddedModule {
                    span: MietteSpan::from(span),
                });
                return Spanned::new(Expr::BytesLit { bytes: Vec::new() }, span);
            }
            match read_file_bytes(&path, ctx.module_dir, &span, ctx) {
                Ok(bytes) => Expr::BytesLit { bytes },
                Err(e) => {
                    ctx.errors.push(e);
                    Expr::BytesLit { bytes: Vec::new() }
                }
            }
        }

        // ── Terminais: sem sub-expressões ──
        Expr::IntLit { .. }
        | Expr::FloatLit { .. }
        | Expr::TextLit { .. }
        | Expr::BytesLit { .. }
        | Expr::Unit
        | Expr::Ident { .. }
        | Expr::Hole
        | Expr::VariantQual { .. }
        | Expr::Break
        | Expr::Continue => expr.node,

        // ── Recursão nos filhos ──
        Expr::Apply { callee, args } => Expr::Apply {
            callee: Box::new(walk_expr(*callee, ctx)),
            args: args.into_iter().map(|e| walk_expr(e, ctx)).collect(),
        },

        Expr::TypeAscription { expr, ty } => Expr::TypeAscription {
            expr: Box::new(walk_expr(*expr, ctx)),
            ty,
        },

        Expr::Grouping { inner } => Expr::Grouping {
            inner: Box::new(walk_expr(*inner, ctx)),
        },

        Expr::Tuple { elements } => Expr::Tuple {
            elements: elements.into_iter().map(|e| walk_expr(e, ctx)).collect(),
        },

        Expr::Let { name, ty, value } => Expr::Let {
            name,
            ty,
            value: Box::new(walk_expr(*value, ctx)),
        },

        Expr::LetDestruct { names, value } => Expr::LetDestruct {
            names,
            value: Box::new(walk_expr(*value, ctx)),
        },

        Expr::Lambda {
            patterns,
            body,
            guards,
            with_bindings,
        } => {
            // Patterns podem conter Pattern::Literal(Spanned<Expr>)
            let patterns = patterns.into_iter().map(|p| walk_pattern(p, ctx)).collect();
            Expr::Lambda {
                patterns,
                body: Box::new(walk_expr(*body, ctx)),
                guards: guards.into_iter().map(|g| walk_guard(g, ctx)).collect(),
                with_bindings: with_bindings
                    .into_iter()
                    .map(|w| walk_with_binding(w, ctx))
                    .collect(),
            }
        }

        Expr::Match { scrutinee, arms } => Expr::Match {
            scrutinee: Box::new(walk_expr(*scrutinee, ctx)),
            arms: arms
                .into_iter()
                .map(|arm| MatchArm {
                    pattern: arm.pattern.map(|p| walk_pattern(p, ctx)),
                    guard: arm.guard.map(|g| walk_expr(g, ctx)),
                    body: walk_expr(arm.body, ctx),
                })
                .collect(),
        },

        Expr::Pipe { lhs, rhs } => Expr::Pipe {
            lhs: Box::new(walk_expr(*lhs, ctx)),
            rhs: Box::new(walk_expr(*rhs, ctx)),
        },

        Expr::PipeLimit { lhs, rhs, limit } => Expr::PipeLimit {
            lhs: Box::new(walk_expr(*lhs, ctx)),
            rhs: Box::new(walk_expr(*rhs, ctx)),
            limit: Box::new(walk_expr(*limit, ctx)),
        },

        Expr::PipeFallback { lhs, rhs } => Expr::PipeFallback {
            lhs: Box::new(walk_expr(*lhs, ctx)),
            rhs: Box::new(walk_expr(*rhs, ctx)),
        },

        Expr::ActionCall { callee, args } => Expr::ActionCall {
            callee,
            args: Box::new(walk_expr(*args, ctx)),
        },

        Expr::TypeOf { expr } => Expr::TypeOf {
            expr: Box::new(walk_expr(*expr, ctx)),
        },

        Expr::Return(expr) => Expr::Return(Box::new(walk_expr(*expr, ctx))),

        Expr::Loop { body } => Expr::Loop {
            body: body.into_iter().map(|e| walk_expr(e, ctx)).collect(),
        },

        Expr::Var { name, ty, value } => Expr::Var {
            name,
            ty,
            value: Box::new(walk_expr(*value, ctx)),
        },

        Expr::Reassign { name, value } => Expr::Reassign {
            name,
            value: Box::new(walk_expr(*value, ctx)),
        },

        Expr::Question(expr) => Expr::Question(Box::new(walk_expr(*expr, ctx))),

        Expr::DotAccess { expr, index } => {
            let index = match index {
                DotIndex::Range {
                    start,
                    end,
                    inclusive,
                } => DotIndex::Range {
                    start: Box::new(walk_expr(*start, ctx)),
                    end: Box::new(walk_expr(*end, ctx)),
                    inclusive,
                },
                other => other,
            };
            Expr::DotAccess {
                expr: Box::new(walk_expr(*expr, ctx)),
                index,
            }
        }

        Expr::ListLit { elements } => Expr::ListLit {
            elements: elements.into_iter().map(|e| walk_expr(e, ctx)).collect(),
        },

        Expr::ArrayLit { elements } => Expr::ArrayLit {
            elements: elements.into_iter().map(|e| walk_expr(e, ctx)).collect(),
        },

        Expr::DictLit { entries } => Expr::DictLit {
            entries: entries
                .into_iter()
                .map(|(k, v)| (walk_expr(k, ctx), walk_expr(v, ctx)))
                .collect(),
        },

        Expr::SetLit { elements } => Expr::SetLit {
            elements: elements.into_iter().map(|e| walk_expr(e, ctx)).collect(),
        },

        Expr::RangeLit {
            start,
            step,
            end,
            inclusive,
        } => Expr::RangeLit {
            start: Box::new(walk_expr(*start, ctx)),
            step: Box::new(walk_expr(*step, ctx)),
            end: Box::new(walk_expr(*end, ctx)),
            inclusive,
        },

        Expr::ForIn {
            var_name,
            iterable,
            body,
        } => Expr::ForIn {
            var_name,
            iterable: Box::new(walk_expr(*iterable, ctx)),
            body: body.into_iter().map(|e| walk_expr(e, ctx)).collect(),
        },

        Expr::In { item, collection } => Expr::In {
            item: Box::new(walk_expr(*item, ctx)),
            collection: Box::new(walk_expr(*collection, ctx)),
        },

        Expr::ChannelOp { source, direction, dest } => Expr::ChannelOp {
            source: Box::new(walk_expr(*source, ctx)),
            direction,
            dest: Box::new(walk_expr(*dest, ctx)),
        },

        Expr::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            let arms = arms
                .into_iter()
                .map(|arm| match arm {
                    SelectArm::Channel {
                        channel,
                        bind_name,
                        body,
                    } => SelectArm::Channel {
                        channel: walk_expr(channel, ctx),
                        bind_name,
                        body: walk_expr(body, ctx),
                    },
                    SelectArm::IoRead {
                        handle_expr,
                        read_mode,
                        bind_name,
                        body,
                    } => SelectArm::IoRead {
                        handle_expr: walk_expr(handle_expr, ctx),
                        read_mode: match read_mode {
                            ReadMode::Chunk(chunk) => ReadMode::Chunk(walk_expr(chunk, ctx)),
                            ReadMode::Line => ReadMode::Line,
                        },
                        bind_name,
                        body: walk_expr(body, ctx),
                    },
                })
                .collect();
            Expr::Select {
                arms,
                timeout_ms: timeout_ms.map(|t| Box::new(walk_expr(*t, ctx))),
                timeout_body: timeout_body.map(|t| Box::new(walk_expr(*t, ctx))),
            }
        }

        Expr::Block { stmts } => Expr::Block {
            stmts: stmts.into_iter().map(|e| walk_expr(e, ctx)).collect(),
        },
    };

    Spanned::new(node, span)
}

/// Walker sobre Pattern — recursa em `Pattern::Literal(Spanned<Expr>)`.
fn walk_pattern(pattern: Spanned<Pattern>, ctx: &mut EmbedCtx) -> Spanned<Pattern> {
    let span = pattern.span;
    let node = match pattern.node {
        Pattern::Literal(expr) => Pattern::Literal(walk_expr(expr, ctx)),
        Pattern::Variant {
            enum_name,
            variant,
            payload,
        } => {
            let payload =
                payload.map(|subs| subs.into_iter().map(|p| walk_pattern(p, ctx)).collect());
            Pattern::Variant {
                enum_name,
                variant,
                payload,
            }
        }
        Pattern::Tuple(elements) => {
            Pattern::Tuple(elements.into_iter().map(|p| walk_pattern(p, ctx)).collect())
        }
        Pattern::Cons { head, tail } => Pattern::Cons {
            head: Box::new(walk_pattern(*head, ctx)),
            tail: Box::new(walk_pattern(*tail, ctx)),
        },
        // Ident, TypedIdent, Wildcard, Nil — sem Expr, retornam self.
        p @ (Pattern::Ident(_) | Pattern::TypedIdent { .. } | Pattern::Wildcard | Pattern::Nil) => {
            p
        }
    };
    Spanned::new(node, span)
}

// ── I/O: ler arquivos ─────────────────────────────────────────────

fn resolve_path(path: &str, module_dir: &Path) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        module_dir.join(p)
    }
}

fn read_file(
    path: &str,
    module_dir: &Path,
    span: &kata_ast::Span,
    ctx: &mut EmbedCtx,
) -> Result<String, EmbedError> {
    let resolved = resolve_path(path, module_dir);
    if Path::new(path).is_absolute() {
        eprintln!(
            "[resolution] warning: @embed_text com path absoluto \"{path}\" — builds não-reprodutíveis"
        );
    }
    match std::fs::read_to_string(&resolved) {
        Ok(content) => {
            ctx.deps.push(resolved);
            Ok(content)
        }
        Err(e) => Err(EmbedError::EmbedFailed {
            path: path.to_string(),
            message: e.to_string(),
            span: MietteSpan::from(*span),
        }),
    }
}

fn read_file_bytes(
    path: &str,
    module_dir: &Path,
    span: &kata_ast::Span,
    ctx: &mut EmbedCtx,
) -> Result<Vec<u8>, EmbedError> {
    let resolved = resolve_path(path, module_dir);
    if Path::new(path).is_absolute() {
        eprintln!(
            "[resolution] warning: @embed_bytes com path absoluto \"{path}\" — builds não-reprodutíveis"
        );
    }
    match std::fs::read(&resolved) {
        Ok(bytes) => {
            ctx.deps.push(resolved);
            Ok(bytes)
        }
        Err(e) => Err(EmbedError::EmbedFailed {
            path: path.to_string(),
            message: e.to_string(),
            span: MietteSpan::from(*span),
        }),
    }
}
