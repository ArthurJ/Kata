//! Testes E2E de closure escape — closures que escapam via return de função nomeada.
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//!
//! Antes da ABI uniformizada (box_ptr sempre presente), closures que escapavam
//! via return de função nomeada causavam SIGSEGV: o box_ptr não era alocado
//! no escopo onde as captures existiam, e o call site não tinha info para
//! distinguir box_ptr de fn_ptr.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_comptime::run_comptime_pass;
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_tree_shaking::tree_shake;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = run_comptime_pass(tree_shake(typed.inner), &resolved.enum_registry, leak_rt_ptr())
        .expect("comptime deve succeed");
    let typed = kata_monomorph::MonoModule::from(typed);
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Combina prelude + módulo do usuário (replica do driver com merge completo).
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

    let mut refined_decls = prelude.refined_decls;
    refined_decls.extend(user.refined_decls);

    let mut enum_pred_decls = prelude.enum_pred_decls;
    enum_pred_decls.extend(user.enum_pred_decls);

    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);

    let mut refines_registry = prelude.refines_registry;
    refines_registry.merge(user.refines_registry);

    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
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
    }
}

#[test]
fn closure_escape_via_return_de_funcao_nomeada() {
    // `make_adder n` retorna um lambda que captura `n`.
    // Antes da ABI uniformizada, isto causava SIGSEGV: o call site de `add5 3`
    // não tinha box_ptr (captures não registradas para Closure no Let arm).
    let src = r#"
make_adder :: Int => (Int -> Int)
lambda n: lambda x: + x n

constant add5 := make_adder 5
constant result := add5 3
result
"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(untag_smi(raw), 8);
    assert_eq!(ty, Ty::int());
}

#[test]
fn closure_sem_capture_funciona() {
    // Lambda sem captures também deve funcionar (box_ptr com n_captures=0).
    let src = r#"
f :: Int => Int
lambda x: + x 1
constant result := f 41
result
"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(untag_smi(raw), 42);
    assert_eq!(ty, Ty::int());
}

#[test]
fn closure_escape_encadeado() {
    // Closures encadeadas: make_adder retorna lambda que captura n,
    // e o resultado é usado como função via constant.
    let src = r#"
make_adder :: Int => (Int -> Int)
lambda n: lambda x: + x n

constant add5 := make_adder 5
constant add10 := make_adder 10
constant a := add5 3
constant b := add10 3
+ a b
"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(untag_smi(raw), 21); // (3+5) + (3+10) = 8 + 13
    assert_eq!(ty, Ty::int());
}

#[test]
fn lambda_com_type_annotation_no_param() {
    // `lambda x::Int: + x 4` — parser deve produzir Pattern::TypedIdent,
    // typeck resolve Int, codegen binda como Ident normal.
    let src = r#"
f :: Int => Int
lambda x::Int: + x 4
constant result := f 4
result
"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(untag_smi(raw), 8);
    assert_eq!(ty, Ty::int());
}

#[test]
fn lambda_com_type_annotation_multiplos_params() {
    // Múltiplos params com type annotation.
    let src = r#"
f :: Int Int => Int
lambda x::Int y::Int: + x y
constant result := f 3 4
result
"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(untag_smi(raw), 7);
    assert_eq!(ty, Ty::int());
}
