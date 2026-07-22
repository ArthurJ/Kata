//! Testes E2E do açúcar sintático `T?` em posição de tipo.
//!
//! `T?` desaçuca para `Result::(T, Err)` onde `Err = Text` (D13 do PRD-refines).
//! É açúcar puro de sintaxe de tipo — não cria subtyping, não cria Ok implícito,
//! não muda o operador `?` de runtime (que continua exclusivo de Actions).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//!
//! DoDs cobertos:
//! - DoD 11: `PositiveInt?` em assinatura é açúcar para `Result::(PositiveInt, Err)`
//! - DoD 12: `Int` não satisfaz `=> Int?` sem wrap explícito (sem Ok implícito)
//! - DoD 13: `?` em runtime continua sendo desempacotamento de Result

use kata_codegen::jit_eval;
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

/// Helper: constrói o Ty esperado para `T?` = `Result::(T, Text)`.
fn result_text_ty(inner: Ty) -> Ty {
    Ty::Generic("Result".into(), vec![inner, Ty::Prim(PrimTy::Text)])
}

// ── DoD 11: `T?` em assinatura é açúcar para `Result::(T, Err)` ──

/// `Int?` em assinatura de action é equivalente a `Result::(Int, Text)`.
/// A action declara `=> Int?` e retorna `Result::Ok 0`.
/// O typeck unifica o tipo do body com o tipo declarado (resolved do açúcar).
#[test]
fn t_question_desugar_int_retorno() {
    let src = r#"action ok42 => Int?
    Result::Ok 0
ok42!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, result_text_ty(Ty::Prim(PrimTy::Int)));
    assert_eq!(raw & 1, 0, "esperado ponteiro (Sum), não SMI");
}

/// `PositiveInt?` em assinatura é equivalente a `Result::(PositiveInt, Text)`.
/// Combina açúcar `T?` com tipo refined.
#[test]
fn t_question_desugar_refined_retorno() {
    let src = r#"data (Int, > _ 0) as PositiveInt
action ok_pos => PositiveInt?
    Result::Ok (5::PositiveInt)
ok_pos!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, result_text_ty(Ty::Struct("PositiveInt".into())));
    assert_eq!(raw & 1, 0, "esperado ponteiro (Sum), não SMI");
}

/// `Int?` e `Result::(Int, Text)` produzem o mesmo tipo após resolution.
/// Ambas as actions retornam o mesmo tipo — confirma que `?` é açúcar puro.
#[test]
fn t_question_desugar_equivalente_result_explicito() {
    // Versão explícita
    let src_explicit = r#"action ok42 => Result::(Int, Text)
    Result::Ok 0
ok42!()"#;
    let (_, ty_explicit) = eval_src(src_explicit);

    // Versão açúcar
    let src_sugar = r#"action ok42 => Int?
    Result::Ok 0
ok42!()"#;
    let (_, ty_sugar) = eval_src(src_sugar);

    assert_eq!(
        ty_explicit, ty_sugar,
        "Int? e Result::(Int, Text) devem produzir o mesmo tipo"
    );
}

// ── DoD 11: `T?` em campo de struct ──

/// Campo de struct com tipo `T?` é resolvido como `Result::(T, Text)`.
/// A struct `Caixa` tem campo `valor :: Int?` (= `Result::(Int, Text)`).
/// Verificamos que o resolution desaçuca `Int?` corretamente:
/// o construtor de `Caixa` é registrado com param `Result::(Int, Text)`.
/// Não fazemos JIT — o dispatch de struct constructors não unifica
/// type params pendentes (bug pré-existente, não relacionado ao `T?`).
#[test]
fn t_question_in_field() {
    let src = "data Caixa (valor :: Int?)\n0";
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    // Se o resolution de `Int?` estivesse errado, o struct_registry
    // teria o tipo do campo errado e o construtor não seria registrado.
    // O infer succeed prova que o tipo foi resolvido corretamente.
    // Procuramos o construtor de Caixa no dispatch table do typed module.
    let caixa_ctor = typed
        .functions
        .iter()
        .find(|f| f.name == "Caixa")
        .expect("construtor de Caixa deve existir");
    // O param do construtor deve ser Result::(Int, Text) — o desaçuca de Int?.
    assert_eq!(
        caixa_ctor.param_types.len(),
        1,
        "Caixa tem 1 campo"
    );
    assert_eq!(
        caixa_ctor.param_types[0],
        Ty::Generic("Result".into(), vec![Ty::Prim(PrimTy::Int), Ty::Prim(PrimTy::Text)]),
        "campo valor :: Int? deve desagucar para Result::(Int, Text)"
    );
}

// ── DoD 12: `Int` não satisfaz `=> Int?` sem wrap explícito ──

/// Action que retorna `Int` nu não satisfaz `=> Int?` — sem Ok implícito.
/// A action declara retorno `Int?` mas o body devolve `42` (Int), não
/// `Result::Ok 42`. O typeck deve rejeitar.
#[test]
fn t_question_not_subtype_int_nao_satisfaz() {
    let src = r#"action f => Int?
    42
f!()"#;
    assert!(
        infer_fails(src),
        "action que retorna Int nu não deve satisfazer => Int? (sem Ok implícito)"
    );
}

/// `Text?` em assinatura com body que devolve `Text` nu também falha.
/// Confirma que a regra vale para qualquer tipo, não só Int.
#[test]
fn t_question_not_subtype_text_nao_satisfaz() {
    let src = "action f => Text?\n    \"hello\"\nf!()";
    assert!(
        infer_fails(src),
        "action que retorna Text nu não deve satisfazer => Text? (sem Ok implícito)"
    );
}

// ── DoD 13: `?` em runtime continua sendo desempacotamento de Result ──

/// `?` em Action desempacota Result — não é no-op em não-Result.
/// Aplicar `?` em `Int` (não-Result) é erro de tipo.
#[test]
fn t_question_runtime_nao_eh_noop_em_nao_result() {
    let src = r#"action foo => Int
    let x := 5
    x ?
    0
foo!()"#;
    assert!(
        infer_fails(src),
        "? em Int (não-Result) deve ser erro de tipo — não é no-op"
    );
}

/// `?` em Action desempacota Result::Ok e continua o fluxo.
/// Confirma que `?` runtime funciona como antes — T? não mudou o operador.
#[test]
fn t_question_runtime_desempacota_ok() {
    let src = r#"action extrai => Result::(Int, Text)
    let r := Result::Ok 42
    r ?
    Result::Ok 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, result_text_ty(Ty::Prim(PrimTy::Int)));
    assert_eq!(raw & 1, 0, "esperado ponteiro (Sum), não SMI");
}

/// `T?` pode encadear: `Int??` = `Result::(Result::(Int, Text), Text)`.
#[test]
fn t_question_encadeado() {
    let src = r#"action f => Int??
    Result::Ok (Result::Ok 42)
f!()"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        result_text_ty(result_text_ty(Ty::Prim(PrimTy::Int))),
        "Int?? deve desagucar para Result::(Result::(Int, Text), Text)"
    );
}