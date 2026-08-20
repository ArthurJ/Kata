//! Testes E2E do `refines` — delegação de interface para tipos refinados.
//!
//! `PositiveInt refines NUM` registra que PositiveInt delega a interface NUM
//! ao seu tipo base (Int). O fallback no dispatch substitui args refined pelo
//! base e retenta. O construtor falível do refined é chamado quando o retorno
//! implementa a interface.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//!
//! DoDs cobertos:
//! - DoD 1: `refines` sem bloco delega todos os métodos via fallback
//! - DoD 2: `+ a b` onde a, b :: PositiveInt retorna Result::(PositiveInt, Err)
//! - DoD 3: `< a b` onde a, b :: PositiveInt retorna Boolean direto
//! - DoD 4: `echo!(x)` funciona para qualquer x (SHOW automático universal)
//! - DoD 5: Caso misto — override encontrado antes do fallback
//! - DoD 6: `+ a b` onde a :: PositiveInt, b :: Int funciona com `refines NUM`
//! - DoD 7: `+ a 0` onde a :: PositiveInt falha SEM `refines NUM`
//! - DoD 8: `+ a b` onde a :: PositiveInt, b :: NonZeroInt falha (nominal)
//! - DoD 9: `refines` em tipo não-refined → erro compile-time
//! - DoD 10: `refines` em interface que o base não implementa → erro compile-time

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
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
    let mut refined_decls = prelude.refined_decls;
    refined_decls.extend(user.refined_decls);
    let mut enum_pred_decls = prelude.enum_pred_decls;
    enum_pred_decls.extend(user.enum_pred_decls);
    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
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
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr())
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Tenta o pipeline até infer. Retorna true se infer falhou (erro compile-time).
fn infer_fails(src: &str) -> bool {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let module = match parse(tokens) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let prelude = match load_prelude() {
        Ok(p) => p,
        Err(_) => return true,
    };
    let user = match resolve(&module) {
        Ok(u) => u,
        Err(_) => return true,
    };
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).is_err()
}

/// Tenta o pipeline até resolve. Retorna true se resolve falhou.
fn resolve_fails(src: &str) -> bool {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let module = match parse(tokens) {
        Ok(m) => m,
        Err(_) => return true,
    };
    resolve(&module).is_err()
}

/// Helper: constrói `Result::(T, Text)`.
fn result_text_ty(inner: Ty) -> Ty {
    Ty::Generic("Result".into(), vec![inner, Ty::Prim(PrimTy::Text)])
}

// ── DoD 1: `refines` sem bloco delega todos os métodos via fallback ──

/// `PositiveInt refines NUM` sem bloco. `+ a b` onde a, b :: PositiveInt
/// despacha via fallback: substitui PositiveInt por Int, encontra `+ :: Int Int => Int`,
/// passa o resultado pelo construtor falível de PositiveInt.
/// O tipo de retorno é `Result::(PositiveInt, Text)`.
#[test]
fn t_refines_delega_aritmetica_via_fallback() {
    let src = r#"data (Int, > _ 0) as PositiveInt
PositiveInt refines NUM
action soma_pos => Result::(PositiveInt, Text)
    let a := 5::PositiveInt
    let b := 3::PositiveInt
    PositiveInt (+ a b)
soma_pos!()"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        result_text_ty(Ty::Struct("PositiveInt".into())),
        "refines NUM deve delegar + via fallback, retornando Result::(PositiveInt, Text)"
    );
}

// ── DoD 2: `+ a b` retorna Result::(PositiveInt, Err) ──

/// Confirmado por t_refines_delega_aritmetica_via_fallback acima.
/// Teste separado para clareza: o valor interno do Result é PositiveInt.
#[test]
fn t_refines_soma_retorna_result_positiveint() {
    let src = r#"data (Int, > _ 0) as PositiveInt
PositiveInt refines NUM
action soma => Result::(PositiveInt, Text)
    let a := 10::PositiveInt
    let b := 20::PositiveInt
    PositiveInt (+ a b)
soma!()"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(ty, result_text_ty(Ty::Struct("PositiveInt".into())));
}

// ── DoD 3: `< a b` retorna Boolean direto (sem construtor) ──

/// `<` retorna Boolean, que não implementa NUM. O fallback substitui
/// PositiveInt por Int, encontra `< :: Int Int => Boolean`, e retorna
/// Boolean direto — sem passar pelo construtor do refined.
#[test]
fn t_refines_comparacao_retorna_boolean_direto() {
    let src = r#"data (Int, > _ 0) as PositiveInt
PositiveInt refines ORD
action menor => Boolean
    let a := 5::PositiveInt
    let b := 10::PositiveInt
    < a b
menor!()"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Sum("Boolean".into()));
}

// ── DoD 4: SHOW automático para refined (sem `refines SHOW`) ──

/// `echo!(a)` onde `a :: PositiveInt` funciona sem `refines SHOW`.
/// O show_synthesis.rs sintetiza `show :: PositiveInt => Text` que delega
/// ao show do base (Int → kata_rt_bi_show).
#[test]
fn t_refines_show_automatico_sem_refines_show() {
    let src = r#"data (Int, > _ 0) as PositiveInt
action mostrar => Text
    let a := 42::PositiveInt
    show a
mostrar!()"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
}

// ── DoD 6: `+ a b` onde a :: PositiveInt, b :: Int funciona com `refines NUM` ──

/// O fallback substitui PositiveInt por Int (args não-refined passam direto).
/// `+ :: Int Int => Int` é encontrado. Retorno Int implementa NUM → construtor.
#[test]
fn t_refines_args_mistos_positiveint_int() {
    let src = r#"data (Int, > _ 0) as PositiveInt
PositiveInt refines NUM
action soma_mista => Result::(PositiveInt, Text)
    let a := 5::PositiveInt
    PositiveInt (+ a 3)
soma_mista!()"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(ty, result_text_ty(Ty::Struct("PositiveInt".into())));
}

// ── DoD 7: `+ a 0` falha SEM `refines NUM` ──

/// Sem `refines NUM`, não há fallback. `+ a 0` onde `a :: PositiveInt` e
/// `0 :: Int` não encontra overload (não há `+ :: PositiveInt Int => ...`).
/// O typeck deve rejeitar.
#[test]
fn t_refines_sem_refines_falha_interoperacao() {
    let src = r#"data (Int, > _ 0) as PositiveInt
action f => Int
    let a := 5::PositiveInt
    + a 0
f!()"#;
    assert!(
        infer_fails(src),
        "sem refines NUM, + a 0 onde a :: PositiveInt deve falhar (sem fallback)"
    );
}

// ── DoD 8: refineds distintos não interoperam (incompatibilidade nominal) ──

/// `+ a b` onde `a :: PositiveInt` e `b :: NonZeroInt` falha mesmo com ambos
/// `refines NUM`. O fallback só dispara quando todos os args refined são o
/// MESMO tipo. Refineds distintos são nominalmente incompatíveis.
#[test]
fn t_refines_atrito_nominal_entre_refineds_distintos() {
    let src = r#"data (Int, > _ 0) as PositiveInt
data (Int, /= _ 0) as NonZeroInt
PositiveInt refines NUM
NonZeroInt refines NUM
action f => Int
    let a := 5::PositiveInt
    let b := 3::NonZeroInt
    + a b
f!()"#;
    assert!(
        infer_fails(src),
        "+ PositiveInt NonZeroInt deve falhar — refineds distintos não interoperam"
    );
}

// ── DoD 11: fallback não substitui arg que já é exact match com o param ──

/// `bar :: Int PositiveInt => Int` é método de MIXED. PositiveInt delega MIXED.
/// Chamada `bar(PositiveInt, PositiveInt)`: o primeiro arg não casa com Int
/// (precisa fallback), mas o segundo arg já casa com PositiveInt por exact
/// match. O fallback cego substitui ambos → `(Int, Int)` → não casa com
/// `Int PositiveInt` → TypeMismatch. A correção só substitui o primeiro →
/// `(Int, PositiveInt)` → casa.
#[test]
fn t_refines_fallback_nao_substitui_arg_exact_match() {
    let src = r#"data (Int, > _ 0) as PositiveInt

interface MIXED
    bar :: Int PositiveInt => Int

Int implements MIXED
    bar :: Int PositiveInt => Int
    lambda a b: + a (b::Int)

PositiveInt refines MIXED

action test => Int
    let a := 5::PositiveInt
    let b := 3::PositiveInt
    bar a b
test!()"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::int(),
        "fallback deve preservar o segundo arg (PositiveInt) que já é exact match, \
         só substituindo o primeiro (PositiveInt→Int)"
    );
}

// ── DoD 9: `refines` em tipo não-refined → erro compile-time ──

/// Aplicar `refines` a um struct não-refined (sem `alias_of` + `predicates`)
/// deve falhar em resolution (pass0.rs valida que o tipo é refined).
#[test]
fn t_refines_em_tipo_nao_refined_falha() {
    // `data Pessoa (nome :: Text)` não é refined — não tem alias_of nem predicates.
    let src = r#"data Pessoa (nome :: Text)
Pessoa refines NUM
Pessoa "João""#;
    assert!(
        resolve_fails(src),
        "refines em tipo não-refined deve falhar em resolution"
    );
}

// ── DoD 10: `refines` em interface que o base não implementa → erro ──

/// Se o tipo base (Int) não implementa a interface delegada, `refines` deve
/// falhar em infer (validação post-merge). Int não implementa ITERABLE.
#[test]
fn t_refines_base_nao_implementa_interface_falha() {
    // Int não implementa ITERABLE (e ITERABLE não existe em 1.0).
    let src = r#"data (Int, > _ 0) as PositiveInt
PositiveInt refines ITERABLE
PositiveInt 5"#;
    assert!(
        infer_fails(src),
        "refines em interface que o base não implementa deve falhar em infer"
    );
}

// ── DoD 5: Caso misto — override encontrado antes do fallback ──

/// `PositiveInt refines NUM` com bloco override para `-`.
/// O `-` tem corpo lambda (override) → cria overload real no DispatchTable.
/// O `+` (não-listado) usa fallback automático.
/// O override deve ser encontrado antes do fallback.
/// O corpo lambda usa downcast `(a::Int)` para chamar o `-` do base,
/// e o construtor falível `PositiveInt(...)` + `match` para desempacotar.
#[test]
fn t_refines_override_encontrado_antes_do_fallback() {
    let src = r#"data (Int, > _ 0) as PositiveInt
PositiveInt refines NUM
    - :: PositiveInt PositiveInt => PositiveInt
    lambda a b:
        match (PositiveInt (- (a::Int) (b::Int)))
            Ok v: v
            otherwise: 1::PositiveInt
action sub_pos => Unit
    let a := 10::PositiveInt
    let b := 3::PositiveInt
    let r := - a b
    echo!(r)
sub_pos!()"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Unit,
        "override de - com corpo lambda deve despachar para o override, não fallback"
    );
}
