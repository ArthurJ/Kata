//! Typeck tests for binding ascription + widening de interface.
//!
//! PRD: docs/PRDs/PRD-binding-ascription-widening.md

use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

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
        type_graph: {
            let mut tg = prelude.type_graph.clone();
            tg.merge(&user.type_graph);
            tg
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
        embed_dependencies: Vec::new(),
    }
}

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
}

fn assert_type_mismatch(err: kata_diagnostics::MiddleError) {
    assert!(
        matches!(err, kata_diagnostics::MiddleError::TypeMismatch { .. }),
        "esperava TypeMismatch, obtive: {err:?}"
    );
}

// ── §6.2: Typeck — tipo concreto ──────────────────────────────

#[test]
fn let_ascription_int_ok() {
    let src = "action main\n    let x::Int := 42\n    echo!(x)\nmain!()";
    infer_src(src);
}

#[test]
fn let_ascription_int_float_rejeitado() {
    let src = "action main\n    let x::Int := 3.14\n    echo!(x)\nmain!()";
    assert_type_mismatch(infer_src_err(src));
}

#[test]
fn var_ascription_int_preserva_tipo() {
    let src = "action main\n    var x::Int := 42\n    var x := + x 1\n    echo!(x)\nmain!()";
    infer_src(src);
}

#[test]
fn var_ascription_int_rebinding_divergente_rejeitado() {
    let src = "action main\n    var x::Int := 42\n    var x := \"hello\"\n    echo!(x)\nmain!()";
    assert_type_mismatch(infer_src_err(src));
}

// ── §6.3: Typeck — widening de interface ──────────────────────

#[test]
fn var_ascription_num_int_ok() {
    let src = "action main\n    var z::NUM := 0\n    echo!(\"ok\")\nmain!()";
    infer_src(src);
}

#[test]
fn var_ascription_num_float_ok() {
    let src = "action main\n    var z::NUM := 3.14\n    echo!(\"ok\")\nmain!()";
    infer_src(src);
}

#[test]
fn var_ascription_num_text_rejeitado() {
    let src = "action main\n    var z::NUM := \"hello\"\n    echo!(z)\nmain!()";
    assert_type_mismatch(infer_src_err(src));
}

#[test]
fn var_ascription_num_preserva_interface_rebinding() {
    let src = "action main\n    var z::NUM := 0\n    var z := 3.14\n    echo!(z)\nmain!()";
    infer_src(src);
}

#[test]
fn var_ascription_num_rebinding_ord_rejeitado() {
    let src =
        "action main\n    var z::NUM := 0\n    var z::ORD := \"hello\"\n    echo!(z)\nmain!()";
    assert_type_mismatch(infer_src_err(src));
}

#[test]
fn var_ascription_num_rebinding_mesma_ascription_ok() {
    let src = "action main\n    var z::NUM := 0\n    var z::NUM := 42\n    echo!(\"ok\")\nmain!()";
    infer_src(src);
}

// ── §3.5: Ascription de interface não é conversão ─────────────

#[test]
fn let_ascription_num_sem_downcast() {
    let src = "action main\n    let z::NUM := 0\n    let n::Int := z\n    echo!(n)\nmain!()";
    assert_type_mismatch(infer_src_err(src));
}

// ── §6.4: Sem ascription (regressão) ──────────────────────────

#[test]
fn var_sem_ascription_preserva_tipo() {
    let src = "action main\n    var x := 42\n    var x := + x 1\n    echo!(x)\nmain!()";
    infer_src(src);
}

#[test]
fn var_sem_ascription_rebinding_divergente_rejeitado() {
    let src = "action main\n    var x := 42\n    var x := \"hello\"\n    echo!(x)\nmain!()";
    assert_type_mismatch(infer_src_err(src));
}

#[test]
fn let_sem_ascription_ok() {
    let src = "action main\n    let x := 42\n    echo!(x)\nmain!()";
    infer_src(src);
}

// ── §4.2: Re-binding com ascription diferente é erro ──────────

#[test]
fn var_ascription_num_rebinding_int_rejeitado() {
    let src = "action main\n    var x::NUM := 0\n    var x::Int := 42\n    echo!(x)\nmain!()";
    assert_type_mismatch(infer_src_err(src));
}

// ── Widening + despacho: tipo concreto preservado ─────────────
// var z::NUM := 0 guarda Int como .ty (para despacho) e NUM como
// .declared_ty (para validação de re-binding).

#[test]
fn var_ascription_num_dispatch_plus_ok() {
    // + z 1 despacha via + :: Int Int => Int (concreto preservado)
    let src = "action main\n    var z::NUM := 0\n    echo!(+ z 1)\nmain!()";
    infer_src(src);
}

#[test]
fn var_ascription_num_dispatch_echo_ok() {
    // echo!(z) despacha via SHOW de Int (concreto preservado)
    let src = "action main\n    var z::NUM := 0\n    echo!(z)\nmain!()";
    infer_src(src);
}

#[test]
fn var_ascription_num_dispatch_eq_ok() {
    // = z 0 despacha via = :: Int Int => Boolean
    let src = "action main\n    var z::NUM := 0\n    echo!(= z 0)\nmain!()";
    infer_src(src);
}

#[test]
fn var_ascription_num_rebinding_float_dispatch_ok() {
    // Após re-binding com Float, despacho usa Float
    let src = "action main\n    var z::NUM := 0\n    var z := 3.14\n    echo!(+ z 1.0)\nmain!()";
    infer_src(src);
}

#[test]
fn var_ascription_num_rebinding_text_rejeitado() {
    // Re-binding sem ascription: Text não implementa NUM
    let src = "action main\n    var z::NUM := 0\n    var z := \"hello\"\n    echo!(z)\nmain!()";
    assert_type_mismatch(infer_src_err(src));
}

#[test]
fn var_ascription_num_reassign_float_ok() {
    // Reassign com widening: z := 3.14 valida contra NUM (declared)
    let src = "action main\n    var z::NUM := 0\n    z := 3.14\n    echo!(\"ok\")\nmain!()";
    infer_src(src);
}

#[test]
fn var_ascription_num_reassign_text_rejeitado() {
    // Reassign com widening: Text não implementa NUM
    let src = "action main\n    var z::NUM := 0\n    z := \"hello\"\n    echo!(\"ok\")\nmain!()";
    assert_type_mismatch(infer_src_err(src));
}

#[test]
fn var_ascription_num_reassign_int_ok() {
    // Reassign com mesmo tipo concreto: OK
    let src = "action main\n    var z::NUM := 0\n    z := 42\n    echo!(\"ok\")\nmain!()";
    infer_src(src);
}
