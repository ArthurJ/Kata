//! Aplicação de diretivas (hooks Enter/Exit) em `LambdaClause` (funções puras).
//!
//! Desugar anotativo: o body NÃO é reescrito. Enter hooks populam
//! `synthetic_pre` e Exit hooks populam `synthetic_post`. O body original
//! permanece inalterado, preservando `tail_pos` para o typeck.

use kata_ast::{Expr, LambdaClause, Spanned};
use kata_resolution::{CustomDirectiveApp, DirectiveDef, DirectiveRegistry, Hook, Target};

use super::reflection::{
    ReflectionInfo, action_stmts_to_exprs, synthesize_args_binding,
    synthesize_static_bindings,
};

/// Aplica diretivas customizadas a uma `LambdaClause` de `Item::Sig`.
///
/// Desugar anotativo: popula `synthetic_pre` (Enter) e `synthetic_post` (Exit)
/// em vez de reescrever `body`. O body original é preservado inalterado.
pub(super) fn apply_directives_to_lambda_clause(
    clause: LambdaClause,
    custom_apps: &[CustomDirectiveApp],
    refl: &ReflectionInfo,
    registry: &DirectiveRegistry,
) -> LambdaClause {
    let mut synthetic_pre: Vec<Spanned<Expr>> = Vec::new();
    let mut synthetic_post: Vec<Spanned<Expr>> = Vec::new();

    // Processar em ordem normal (primeira = mais externa = executa primeiro).
    // No approach anterior (rev + envolvimento), a mais externa envolvia tudo
    // e seus bindings executavam primeiro. Aqui, append em ordem normal
    // produz a mesma ordem de execução.
    for app in custom_apps {
        let defs: Vec<&DirectiveDef> = registry
            .lookup_by_name(&app.name)
            .into_iter()
            .filter(|d| matches!(d.key.on, Target::Function | Target::Any))
            .filter(|d| d.key.arg_keys.as_slice() == app.arg_keys.as_slice())
            .filter(|d| app.site_when.is_none_or(|w| d.key.when == w))
            .collect();

        if defs.is_empty() {
            continue;
        }

        for def in &defs {
            match def.key.when {
                Hook::Enter => {
                    synthetic_pre.extend(build_synthetic_pre(def, refl, app));
                }
                Hook::Exit => {
                    synthetic_post.extend(build_synthetic_post(def, refl, app));
                }
                Hook::ShortCircuit | Hook::Transform => {
                    // ShortCircuit e Transform não podem decorar funções — o
                    // resolution já rejeitou a combinação. Ignorar.
                }
            }
        }
    }

    LambdaClause {
        patterns: clause.patterns,
        body: clause.body,
        synthetic_pre,
        synthetic_post,
        guards: clause.guards,
        with_bindings: clause.with_bindings,
    }
}

/// Enter em função pura: produz código para `synthetic_pre`.
///
/// Contém: bindings estáticos + _args + args do site + statements da diretiva.
/// O codegen/interp avalia isto antes do body.
fn build_synthetic_pre(
    def: &DirectiveDef,
    refl: &ReflectionInfo,
    app: &CustomDirectiveApp,
) -> Vec<Spanned<Expr>> {
    let mut stmts = Vec::new();

    // Bindings estáticos
    stmts.extend(synthesize_static_bindings(refl));

    // _args binding
    stmts.push(synthesize_args_binding(refl));

    // Args do site de aplicação (let _msg := "..." etc.)
    stmts.extend(super::action_hooks::synthesize_site_arg_bindings(app));

    // Statements do body da diretiva
    stmts.extend(action_stmts_to_exprs(&def.body));

    stmts
}

/// Exit em função pura: produz código para `synthetic_post`.
///
/// Contém: bindings estáticos + _args + args do site + statements da diretiva.
/// O `_return` é bindado pelo codegen/interp ao resultado do body — não pelo
/// desugar. O typeck declara `_return` no escopo ao inferir `synthetic_post`.
fn build_synthetic_post(
    def: &DirectiveDef,
    refl: &ReflectionInfo,
    app: &CustomDirectiveApp,
) -> Vec<Spanned<Expr>> {
    let mut stmts = Vec::new();

    // Bindings estáticos
    stmts.extend(synthesize_static_bindings(refl));

    // _args binding
    stmts.push(synthesize_args_binding(refl));

    // Args do site de aplicação (let _msg := "..." etc.)
    stmts.extend(super::action_hooks::synthesize_site_arg_bindings(app));

    // Statements do body da diretiva (referenciam _return, que é bindado
    // pelo codegen/interp ao resultado do body)
    stmts.extend(action_stmts_to_exprs(&def.body));

    stmts
}