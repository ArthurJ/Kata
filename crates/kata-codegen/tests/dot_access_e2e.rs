//! Testes E2E de codegen de DotAccess (field access + index access).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

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

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// `pessoa.nome` retorna "João" — field access primeiro campo.
/// O valor é um ponteiro para texto na arena; o JIT retorna o ptr como i64.
#[test]
fn field_access_primeiro_campo_retorna_text() {
    let src = "data Pessoa (nome::Text idade::Int)\nconstant p := Pessoa \"João\" 30\np.nome";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    // Text é ponteiro na arena — não é SMI. Apenas verifica que não panica.
    let _ = raw;
}

/// `pessoa.idade` retorna 30 (SMI) — field access segundo campo.
#[test]
fn field_access_segundo_campo_retorna_int() {
    let src = "data Pessoa (nome::Text idade::Int)\nconstant p := Pessoa \"João\" 30\np.idade";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 30);
}

/// `(10, 20, 30).0` retorna 10 — index access primeiro elemento.
#[test]
fn index_access_primeiro_retorna_10() {
    let src = "(10, 20, 30).0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 10);
}

/// `(10, 20, 30).1` retorna 20 — index access segundo elemento.
#[test]
fn index_access_segundo_retorna_20() {
    let src = "(10, 20, 30).1";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 20);
}

/// `(10, 20, 30).2` retorna 30 — index access terceiro elemento.
#[test]
fn index_access_terceiro_retorna_30() {
    let src = "(10, 20, 30).2";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 30);
}

/// `(10, 20, 30).(-1)` retorna 30 — index access negativo (último).
#[test]
fn index_access_negativo_ultimo_retorna_30() {
    let src = "(10, 20, 30).(-1)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 30);
}

/// `(10, 20, 30).(-2)` retorna 20 — index access negativo (penúltimo).
#[test]
fn index_access_negativo_penultimo_retorna_20() {
    let src = "(10, 20, 30).(-2)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 20);
}

/// `(10, 20, 30).(-3)` retorna 10 — index access negativo (primeiro).
#[test]
fn index_access_negativo_primeiro_retorna_10() {
    let src = "(10, 20, 30).(-3)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 10);
}

/// Struct aninhada: `pessoa.endereco.rua` retorna "Rua A" — field access encadeado.
#[test]
fn field_access_encadeado_retorna_text() {
    let src = "data Endereco (rua::Text cidade::Text)\ndata Pessoa (nome::Text end::Endereco)\nconstant p := Pessoa \"João\" (Endereco \"Rua A\" \"Cidade B\")\np.end.rua";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// Struct aninhada: `pessoa.endereco.cidade` retorna "Cidade B".
#[test]
fn field_access_encadeado_segundo_campo() {
    let src = "data Endereco (rua::Text cidade::Text)\ndata Pessoa (nome::Text end::Endereco)\nconstant p := Pessoa \"João\" (Endereco \"Rua A\" \"Cidade B\")\np.end.cidade";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// Tupla de 1 elemento: `(42,).0` retorna 42.
#[test]
fn index_access_tupla_um_elemento() {
    let src = "(42,).0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 42);
}

/// Tupla heterogênea: `(42, "ok").0` retorna 42 (Int).
#[test]
fn index_access_tupla_heterogenea_primeiro() {
    let src = "(42, \"ok\").0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 42);
}

/// Struct com 1 campo: `w.valor` retorna 42.
#[test]
fn struct_um_campo_field_access_retorna_42() {
    let src = "data Wrapper (valor::Int)\nconstant w := Wrapper 42\nw.valor";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 42);
}

/// Field access seguido de aritmética: `p.idade + 1` retorna 31.
#[test]
fn field_access_em_expressao_aritmetica() {
    let src = "data Pessoa (nome::Text idade::Int)\nconstant p := Pessoa \"João\" 30\n+ p.idade 1";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 31);
}
