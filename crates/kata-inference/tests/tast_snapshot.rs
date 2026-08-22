//! Snapshot tests da TAST (Typed AST) para programas Kata simples.
//!
//! Cada teste compila um programa pequeno até a fase de inferência e
//! faz snapshot do `format!("{:#?}", typed_module)`.
//!
//! Para aceitar mudanças: `cargo insta accept` (ou `INSTA_UPDATE=always cargo test`).

use kata_core::ty::TypeEnv;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

/// Executa o pipeline até inferência e retorna um resumo determinístico da TAST.
///
/// O `Debug` da TAST inteira não é determinístico (DispatchTable usa HashMap).
/// Aqui extraímos apenas as partes relevantes e determinísticas:
/// - Tipos das funções nomeadas (ordenadas por nome)
/// - Tipos das actions (ordenadas por nome)
/// - Tipo do entry point
/// - Kind do entry point
fn tast_snapshot(src: &str) -> String {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");

    let mut out = String::new();

    // Entry point
    out.push_str("=== Entry ===\n");
    out.push_str(&format!("  ty: {:?}\n", typed.entry.node.ty));
    out.push_str(&format!("  kind: {:?}\n", typed.entry.node.kind));

    // Funções do usuário (não do prelude) — ordenadas por nome
    let mut user_fns: Vec<_> = typed
        .functions
        .iter()
        .filter(|f| !f.name.starts_with("__kata"))
        .collect();
    user_fns.sort_by(|a, b| a.name.cmp(&b.name));
    if !user_fns.is_empty() {
        out.push_str("\n=== Functions ===\n");
        for f in &user_fns {
            out.push_str(&format!(
                "  {} :: {:?} => {:?}\n",
                f.name, f.param_types, f.ret_ty
            ));
        }
    }

    // Actions do usuário — ordenadas por nome
    let mut user_actions: Vec<_> = typed
        .actions
        .iter()
        .filter(|a| !a.name.starts_with("__kata"))
        .collect();
    user_actions.sort_by(|a, b| a.name.cmp(&b.name));
    if !user_actions.is_empty() {
        out.push_str("\n=== Actions ===\n");
        for a in &user_actions {
            out.push_str(&format!(
                "  {} :: {:?} => {:?}\n",
                a.name, a.param_types, a.ret_ty
            ));
        }
    }

    out.trim_end().to_string()
}

/// Combina prelude + módulo do usuário (replica do driver).
fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);
    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);
    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry: {
            let mut ir = prelude.interface_registry.clone();
            ir.merge(user.interface_registry.clone());
            ir
        },
        refines_registry: {
            let mut rr = prelude.refines_registry.clone();
            rr.merge(user.refines_registry.clone());
            rr
        },
        functions: {
            let mut fns = prelude.functions;
            let user_fn_names: std::collections::HashSet<&str> =
                user.functions.iter().map(|f| f.name.as_str()).collect();
            fns.retain(|f| !user_fn_names.contains(f.name.as_str()));
            fns.extend(user.functions);
            fns
        },
        actions: {
            let mut acts = prelude.actions;
            let user_action_names: std::collections::HashSet<&str> =
                user.actions.iter().map(|a| a.name.as_str()).collect();
            acts.retain(|a| !user_action_names.contains(a.name.as_str()));
            acts.extend(user.actions);
            acts
        },
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

#[test]
fn tast_aritmetica_simples() {
    let src = "+ 1 2";
    insta::assert_snapshot!(tast_snapshot(src));
}

#[test]
fn tast_let_binding() {
    let src = "constant x := 42
x";
    insta::assert_snapshot!(tast_snapshot(src));
}

#[test]
fn tast_lambda_clauses() {
    let src = r#"dobro :: Int => Int
lambda x: + x x

action main
    let r := dobro 21
    echo!(r)

main!()"#;
    insta::assert_snapshot!(tast_snapshot(src));
}

#[test]
fn tast_destructuring_tupla() {
    let src = r#"action main
    let (x, y) := (1, 2)
    echo!(x)
    echo!(y)

main!()"#;
    insta::assert_snapshot!(tast_snapshot(src));
}

#[test]
fn tast_cons_pattern() {
    let src = r#"head :: [Int] => Optional::(Int)
lambda []: None
lambda [h:_]: Some h

action main
    let r := head [10 20 30]
    echo!(show r)

main!()"#;
    insta::assert_snapshot!(tast_snapshot(src));
}
