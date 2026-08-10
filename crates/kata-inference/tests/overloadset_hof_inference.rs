//! Testes de inference de HOFs (map/fold/filter) com OverloadSet.
//!
//! Quando `let f := + _ 2` produz um lambda deferido com tipo OverloadSet,
//! `map f [1 2 3]` deve selecionar a overload correta pelo elem_ty da coleção.

use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

// ── Helpers ──────────────────────────────────────────────────

fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
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

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
}

fn entry_typed(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

fn entry_kind(tmod: &kata_inference::TypedModule) -> &TypedExprKind {
    &tmod.entry.node.kind
}

// ── Testes: map com OverloadSet ─────────────────────────────

#[test]
fn map_com_ident_overloadset_int() {
    // let f := + _ 2
    // map f [1 2 3]  →  List(Int)
    // f tem tipo OverloadSet { name: "+", overloads: [(Int,Int)→Int, (Int,Float)→Float, ...] }
    // map seleciona (Int,Int)→Int porque elem_ty = Int
    let typed = infer_src("constant f := + _ 2\nmap f [1 2 3]");
    assert_eq!(typed.entry.node.ty, Ty::List(Box::new(Ty::int())));
}

#[test]
fn map_com_ident_overloadset_float() {
    // let f := + _ 2.0
    // map f [1.0 2.0 3.0]  →  List(Float)
    let typed = infer_src("constant f := + _ 2.0\nmap f [1.0 2.0 3.0]");
    assert_eq!(typed.entry.node.ty, Ty::List(Box::new(Ty::float())));
}

// ── Testes: fold com OverloadSet ────────────────────────────

#[test]
fn fold_com_ident_overloadset_int() {
    // let f := + _ _
    // fold f 0 [1 2 3]  →  Int (6)
    let typed = infer_src("constant f := + _ _\nfold f 0 [1 2 3]");
    assert_eq!(typed.entry.node.ty, Ty::int());
}

// ── Testes: map inline (não-OverloadSet) continua funcionando ─

#[test]
fn map_com_lambda_inline_continua_funcionando() {
    // map (+ 10 _) [1 2 3] — lambda inline recebe hint, não produz OverloadSet
    let typed = infer_src("map (+ 10 _) [1 2 3]");
    assert_eq!(typed.entry.node.ty, Ty::List(Box::new(Ty::int())));
}

#[test]
fn map_com_hole_soma_continua_funcionando() {
    let typed = infer_src("map (+ _ 10) [1 2 3]");
    assert_eq!(typed.entry.node.ty, Ty::List(Box::new(Ty::int())));
}