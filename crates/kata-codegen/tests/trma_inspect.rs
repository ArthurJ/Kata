//! Inspeção da TAST do examples/trma.kata após optimize.
//! Imprime as funções soma/soma_acc/fatorial/fatorial_acc para confirmar a reescrita TRMA.

use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{load_stdlib_for_tests, resolve};

fn merge_resolved(
    prelude: kata_resolution::ResolvedModule,
    user: kata_resolution::ResolvedModule,
) -> kata_resolution::ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);
    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);
    kata_resolution::ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
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
        type_graph: prelude.type_graph.clone(),
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

/// Serializa uma TypedExpr em formato legível (mini-pretty-printer).
fn dump_expr(expr: &kata_ast::Spanned<kata_inference::TypedExpr>, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    match &expr.node.kind {
        TypedExprKind::IntLit { text } => format!("{indent}IntLit({text})"),
        TypedExprKind::Ident { name } => format!("{indent}Ident({name})"),
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            let mut s = format!("{indent}Closure(\n");
            s.push_str(&format!(
                "{indent}  callee: {}\n",
                dump_expr(callee, depth + 2)
            ));
            for (i, arg) in args.iter().enumerate() {
                s.push_str(&format!(
                    "{indent}  arg[{i}]: {}\n",
                    dump_expr(arg, depth + 2)
                ));
            }
            if let Some(ffi) = ffi_symbol {
                s.push_str(&format!("{indent}  ffi: {ffi}\n"));
            }
            s.push_str(&format!("{indent})"));
            s
        }
        TypedExprKind::Match { scrutinee, arms } => {
            let mut s = format!("{indent}Match(\n");
            s.push_str(&format!(
                "{indent}  scrutinee: {}\n",
                dump_expr(scrutinee, depth + 2)
            ));
            for arm in arms {
                let pat_str = match &arm.pattern {
                    Some(p) => format!("{:?}", p.node),
                    None => "otherwise".to_string(),
                };
                s.push_str(&format!(
                    "{indent}  arm {pat_str}: {}\n",
                    dump_expr(&arm.body, depth + 2)
                ));
            }
            s.push_str(&format!("{indent})"));
            s
        }
        TypedExprKind::Grouping { inner } => dump_expr(inner, depth),
        _ => format!("{indent}({:?})", expr.node.kind),
    }
}

#[test]
fn inspect_trma_kata_snapshot() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/trma.kata"
    ))
    .expect("não consegui ler examples/trma.kata");
    let tokens = lex(&src).expect("lex");
    let module = parse(tokens).expect("parse");
    let prelude = load_stdlib_for_tests().expect("prelude");
    let user = resolve(&module).expect("resolve");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer");

    println!("\n=== ANTES do optimize ===");
    for f in &typed.functions {
        if f.name == "soma" || f.name == "fatorial" {
            println!(
                "  {} :: {:?} => {:?}  ({} cláusula(s))",
                f.name,
                f.param_types,
                f.ret_ty,
                f.clauses.len()
            );
        }
    }

    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = typed.inner;

    println!("\n=== DEPOIS do optimize ===");
    for f in &typed.functions {
        if f.name == "soma"
            || f.name == "soma_acc"
            || f.name == "fatorial"
            || f.name == "fatorial_acc"
        {
            println!(
                "\n  {} :: {:?} => {:?}  ({} cláusula(s))",
                f.name,
                f.param_types,
                f.ret_ty,
                f.clauses.len()
            );
            for (i, clause) in f.clauses.iter().enumerate() {
                println!("  --- cláusula {} ---", i);
                for pat in &clause.patterns {
                    println!("    pattern: {:?}", pat.node);
                }
                println!("{}", dump_expr(&clause.body, 2));
            }
        }
    }
}
