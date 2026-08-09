//! Aplicação de diretivas (hooks Enter/Exit) em `LambdaClause` (funções puras).

use kata_ast::{Expr, LambdaClause, Span, Spanned};
use kata_resolution::{DirectiveDef, DirectiveRegistry, Hook, Target};

use super::reflection::{
    ReflectionInfo, action_stmts_to_exprs, synthesize_args_binding, synthesize_return_binding,
    synthesize_static_bindings,
};

/// Aplica diretivas customizadas a uma `LambdaClause` de `Item::Sig`.
pub(super) fn apply_directives_to_lambda_clause(
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
