//! Testes E2E de codegen de loop, break, continue.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr()).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Combina prelude + módulo do usuário (replica do driver).
fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);
    // Merge enum_registry: prelude + user (user enums sobrescrevem prelude).
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

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Loop com break: soma 1 a 5 e retorna o acumulador.
/// O break sai do loop quando i > 5.
#[test]
fn loop_com_break_soma_1_a_5() {
    let src = r#"action soma_loop => Int
    var i := 0
    var acc := 0
    loop
        i := + i 1
        match > i 5
            True: break
            False: acc := + acc i
    acc
soma_loop!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // 1 + 2 + 3 + 4 + 5 = 15
    assert_eq!(untag_smi(raw), 15, "soma_loop deve ser 15 (1+2+3+4+5)");
}

/// Loop com break incondicional: executa uma vez e sai.
#[test]
fn loop_break_incondicional() {
    let src = r#"action loop_una => Int
    var x := 0
    loop
        x := + x 1
        break
    x
loop_una!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        1,
        "loop_una deve ser 1 (break na primeira iteracao)"
    );
}

/// Loop com continue: soma 1 a 5 pulando o 3.
/// Testa break (i > 5) e continue (i == 3) no mesmo loop.
#[test]
fn loop_continue_pula_3() {
    let src = r#"action soma_pulando => Int
    var i := 0
    var acc := 0
    loop
        i := + i 1
        match > i 5
            True: break
            False: match = i 3
                True: continue
                False: acc := + acc i
    acc
soma_pulando!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // 1 + 2 + 4 + 5 = 12 (pula 3 com continue)
    assert_eq!(
        untag_smi(raw),
        12,
        "soma_pulando deve ser 12 (1+2+4+5, pulando 3)"
    );
}
