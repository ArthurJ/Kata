//! Aplicação de diretivas (hooks Enter/Exit) em `LambdaClause` (funções puras).

use kata_ast::{Expr, LambdaClause, Span, Spanned};
use kata_resolution::{CustomDirectiveApp, DirectiveDef, DirectiveRegistry, Hook, Target};

use super::reflection::{
    ReflectionInfo, action_stmts_to_exprs, synthesize_args_binding, synthesize_return_binding,
    synthesize_static_bindings,
};

/// Aplica diretivas customizadas a uma `LambdaClause` de `Item::Sig`.
pub(super) fn apply_directives_to_lambda_clause(
    clause: LambdaClause,
    custom_apps: &[CustomDirectiveApp],
    refl: &ReflectionInfo,
    registry: &DirectiveRegistry,
) -> LambdaClause {
    let mut current_body = clause.body;

    // Processar de dentro para fora.
    for app in custom_apps.iter().rev() {
        // Coletar defs aplicáveis a este item (função) que casam
        // com os arg_keys do site de aplicação e o when do site (se presente).
        let defs: Vec<&DirectiveDef> = registry
            .lookup_by_name(&app.name)
            .into_iter()
            .filter(|d| matches!(d.key.on, Target::Function | Target::Any))
            .filter(|d| d.key.arg_keys.as_slice() == app.arg_keys.as_slice())
            .filter(|d| app.site_when.map_or(true, |w| d.key.when == w))
            .collect();

        if defs.is_empty() {
            continue;
        }

        for def in &defs {
            current_body = apply_hook_to_lambda_body(current_body, def, refl, app);
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
    app: &CustomDirectiveApp,
) -> Spanned<Expr> {
    match def.key.when {
        Hook::Enter => apply_enter_to_lambda_body(body, def, refl, app),
        Hook::Exit => apply_exit_to_lambda_body(body, def, refl, app),
        Hook::ShortCircuit | Hook::Transform => {
            // ShortCircuit e Transform não podem decorar funções — o resolution
            // já rejeitou a combinação. Mas se chegamos aqui, o body da diretiva
            // tem Target::Any ou Target::Function com ShortCircuit/Transform, o que
            // é impossível. Retornar inalterado.
            body
        }
    }
}

/// Enter em função pura: prependa bindings + args do site + statements da diretiva
/// antes do body, envolvendo em `Expr::Block`.
fn apply_enter_to_lambda_body(
    body: Spanned<Expr>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
    app: &CustomDirectiveApp,
) -> Spanned<Expr> {
    let span = Span::synthetic();
    let mut stmts = Vec::new();

    // Bindings estáticos
    stmts.extend(synthesize_static_bindings(refl));

    // _args binding
    stmts.push(synthesize_args_binding(refl));

    // Args do site de aplicação (let _msg := "..." etc.)
    stmts.extend(super::action_hooks::synthesize_site_arg_bindings(app));

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
    app: &CustomDirectiveApp,
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

    // Args do site de aplicação (let _msg := "..." etc.)
    stmts.extend(super::action_hooks::synthesize_site_arg_bindings(app));

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
