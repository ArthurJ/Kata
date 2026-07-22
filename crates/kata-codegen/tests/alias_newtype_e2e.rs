//! Testes E2E de codegen de `alias` (newtype).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Verifica DoD 4: `alias` cria newtype com rigidez nominal e mesmo ABI.

use kata_codegen::jit_eval;
use kata_core::InterfaceRegistry;
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
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// `alias Float as Altura` seguido de `Altura 1.75` produz `Ty::Struct("Altura")`.
/// O valor F64 é retornado como i64 (bitcast), mesmo ABI que Float.
#[test]
fn alias_float_as_altura_constrói() {
    let src = "alias Float as Altura\nAltura 1.75";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct("Altura".into()));
    // Float é retornado como bits F64 reinterpretados como i64.
    let bits = f64::to_bits(1.75) as i64;
    assert_eq!(raw, bits);
}

/// `alias Float as Altura` — construtor aceita literal Float.
/// `42::Float` rebaixa para f64, `Altura (42::Float)` envolve em newtype.
#[test]
fn alias_float_as_altura_com_ascription() {
    let src = "alias Float as Altura\nAltura (42::Float)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct("Altura".into()));
    let bits = f64::to_bits(42.0) as i64;
    assert_eq!(raw, bits);
}

/// Rigidez nominal: `Altura` ≠ `Float` em typeck.
/// Passar `Altura` onde se espera `Float` deve falhar na inferência.
/// Usa infer_module diretamente para capturar o erro.
#[test]
fn alias_altura_nao_igual_float_typeck() {
    let src = "alias Float as Altura\nlet x := Altura 1.75\nx::Float";
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(result.is_err(), "x::Float onde x é Altura deve falhar");
}

/// Alias de struct com campos: `alias Pessoa as Pessoa2`.
/// `Pessoa2 "João" 30` produz `Ty::Struct("Pessoa2")`.
#[test]
fn alias_struct_com_campos_constrói() {
    let src = "data Pessoa (nome::Text idade::Int)\nalias Pessoa as Pessoa2\nlet p := Pessoa2 \"João\" 30\np.idade";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 30);
}

/// Alias de struct: field access funciona no newtype.
#[test]
fn alias_struct_field_access_retorna_text() {
    let src = "data Pessoa (nome::Text idade::Int)\nalias Pessoa as Pessoa2\nlet p := Pessoa2 \"Maria\" 25\np.nome";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw; // Text é ponteiro na arena
}

/// Alias de Int: `alias Int as Counter`.
/// `Counter 5` produz `Ty::Struct("Counter")` com valor SMI 5.
#[test]
fn alias_int_as_counter() {
    let src = "alias Int as Counter\nCounter 5";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct("Counter".into()));
    assert_eq!(untag_smi(raw), 5);
}

/// Alias de alias: `alias Float as Altura` seguido de `alias Altura as AlturaValida`.
/// `AlturaValida (Altura 1.75)` produz `Ty::Struct("AlturaValida")`.
#[test]
fn alias_de_alias() {
    let src = "alias Float as Altura\nalias Altura as AlturaValida\nAlturaValida (Altura 1.75)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct("AlturaValida".into()));
    let bits = f64::to_bits(1.75) as i64;
    assert_eq!(raw, bits);
}

/// Rigidez nominal: `Counter` ≠ `Int` — passar Counter onde espera Int falha.
#[test]
fn alias_counter_nao_igual_int_typeck() {
    let src = "alias Int as Counter\nlet c := Counter 5\nlet f := + _::Int 1\nf c";
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(
        result.is_err(),
        "f c onde f espera Int e c é Counter deve falhar"
    );
}
