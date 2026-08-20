//! Testes E2E de parser arity-aware (Fase 3 — arity-uniformization).
//!
//! Pipeline completo: lex -> parse_with_arity -> resolve -> infer -> optimize -> codegen -> JIT.
//! Valida que o parser arity-aware coleta exatamente N args e permite sub-aplicações.
//!
//! Estes testes usam `parse_with_arity` com aridades construídas manualmente,
//! simulando o que o ciclo de dois passes (Fase 4) produzirá automaticamente.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse_with_arity;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;
use std::collections::HashMap;

/// Executa o pipeline completo com parser arity-aware e retorna o valor JIT + tipo.
fn eval_src_arity(src: &str, arities: HashMap<String, usize>) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse_with_arity(tokens, arities).expect("parse_with_arity deve succeed");
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

/// Combina prelude + módulo do usuário (replica do driver).
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

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Constrói um mapa de aridades com as operações do prelude.
/// `+`, `-`, `*` têm aridade 2 no prelude.
fn prelude_arities() -> HashMap<String, usize> {
    let mut m = HashMap::new();
    m.insert("+".to_string(), 2);
    m.insert("-".to_string(), 2);
    m.insert("*".to_string(), 2);
    m.insert("/".to_string(), 2);
    m.insert("=".to_string(), 2);
    m.insert("!=".to_string(), 2);
    m.insert("<".to_string(), 2);
    m.insert(">".to_string(), 2);
    m.insert("<=".to_string(), 2);
    m.insert(">=".to_string(), 2);
    m.insert("show".to_string(), 1);
    m.insert("len".to_string(), 1);
    m.insert("mod".to_string(), 2);
    m.insert("and".to_string(), 2);
    m.insert("or".to_string(), 2);
    m.insert("not".to_string(), 1);
    m.insert("head".to_string(), 1);
    m.insert("tail".to_string(), 1);
    m.insert("cons".to_string(), 2);
    m.insert("int_to_text".to_string(), 1);
    m.insert("bool_to_text".to_string(), 1);
    m.insert("+".to_string(), 2);
    m.insert("text_replace".to_string(), 2);
    m.insert("rational".to_string(), 1);
    m.insert("float".to_string(), 1);
    m.insert("int".to_string(), 1);
    m.insert("byte".to_string(), 1);
    m.insert("bytes".to_string(), 1);
    m.insert("text".to_string(), 1);
    m.insert("hash".to_string(), 1);
    m.insert("abs".to_string(), 1);
    m.insert("next".to_string(), 1);
    m.insert("at".to_string(), 2);
    m.insert("contains".to_string(), 2);
    m.insert("slice".to_string(), 3);
    m.insert("map".to_string(), 2);
    m.insert("filter".to_string(), 2);
    m.insert("fold".to_string(), 3);
    m.insert("xor".to_string(), 2);
    m.insert(">>".to_string(), 2);
    m.insert("<<".to_string(), 2);
    m.insert("union".to_string(), 2);
    m.insert("intersection".to_string(), 2);
    m.insert("difference".to_string(), 2);
    m.insert("insert".to_string(), 3);
    m.insert("remove".to_string(), 2);
    m.insert("div".to_string(), 2);
    m
}

// ── Sub-aplicação (DoD Fase 3) ─────────────────────────────────

/// `+ 5 * 2 2` deve parsear como `Apply(+, [5, Apply(*, [2, 2])])` e avaliar para 9.
/// O parser arity-aware coleta 2 args para `+`: o 1º é `5` (literal), o 2º é
/// `Apply(*, [2, 2])` (sub-aplicação arity-aware recursiva).
#[test]
fn sub_aplicacao_soma_produto() {
    let src = "+ 5 * 2 2";
    let arities = prelude_arities();
    let (raw, ty) = eval_src_arity(src, arities);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 9, "+ 5 * 2 2 deve ser 9");
}

/// `* 5 + 2 2` deve parsear como `Apply(*, [5, Apply(+, [2, 2])])` e avaliar para 20.
#[test]
fn sub_aplicacao_produto_soma() {
    let src = "* 5 + 2 2";
    let arities = prelude_arities();
    let (raw, ty) = eval_src_arity(src, arities);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 20, "* 5 + 2 2 deve ser 20");
}

/// Aplicação simples com aridade 2 continua funcionando.
/// `+ 1 2` → 3
#[test]
fn aplicacao_simples_aridade_2() {
    let src = "+ 1 2";
    let arities = prelude_arities();
    let (raw, ty) = eval_src_arity(src, arities);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 3, "+ 1 2 deve ser 3");
}

/// Aplicação aninhada com grouping.
/// `+ (+ 1 2) 3` → 6
#[test]
fn aplicacao_aninhada_com_grouping() {
    let src = "+ (+ 1 2) 3";
    let arities = prelude_arities();
    let (raw, ty) = eval_src_arity(src, arities);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 6, "+ (+ 1 2) 3 deve ser 6");
}

/// Função do usuário com aridade conhecida.
/// `soma :: Int Int => Int` + `soma 3 4` → 7
#[test]
fn funcao_usuario_aridade_conhecida() {
    let src = "soma :: Int Int => Int\nlambda a b: + a b\nsoma 3 4";
    let mut arities = prelude_arities();
    arities.insert("soma".to_string(), 2);
    let (raw, ty) = eval_src_arity(src, arities);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 7, "soma 3 4 deve ser 7");
}

/// Função do usuário com sub-aplicação.
/// `soma :: Int Int => Int` + `soma 3 * 4 5` → 3 + 20 = 23
#[test]
fn funcao_usuario_com_sub_aplicacao() {
    let src = "soma :: Int Int => Int\nlambda a b: + a b\nsoma 3 * 4 5";
    let mut arities = prelude_arities();
    arities.insert("soma".to_string(), 2);
    let (raw, ty) = eval_src_arity(src, arities);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 23, "soma 3 * 4 5 deve ser 23");
}

/// Aridade 1: `show 42` → "42" (Text)
#[test]
fn aridade_1_show() {
    let src = "show 42";
    let arities = prelude_arities();
    let (raw, ty) = eval_src_arity(src, arities);
    assert_eq!(ty, Ty::text(), "show 42 deve retornar Text");
    // raw é ponteiro para string do runtime — não decodificamos, só confere o tipo
    let _ = raw;
}

/// `+ 1 2 3` com aridade 2 deve dar erro de parser (excesso posicional).
#[test]
fn excesso_posicional_erro() {
    let src = "+ 1 2 3";
    let arities = prelude_arities();
    let tokens = lex(src).expect("lex deve succeed");
    let result = parse_with_arity(tokens, arities);
    assert!(
        result.is_err(),
        "+ 1 2 3 com aridade 2 deve dar erro de parser (excesso posicional)"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("aridade padrão 2") || err.contains("excesso"),
        "erro deve mencionar aridade padrão 2 ou excesso, got: {err}"
    );
}

/// `+ 1` com aridade 2 deve dar erro de parser (falta argumento).
#[test]
fn falta_argumento_erro() {
    let src = "+ 1";
    let arities = prelude_arities();
    let tokens = lex(src).expect("lex deve succeed");
    let result = parse_with_arity(tokens, arities);
    assert!(
        result.is_err(),
        "+ 1 com aridade 2 deve dar erro de parser (falta argumento)"
    );
}

/// Dois items separados por quebra de linha (StmtSep) continuam funcionando.
/// `+ 1 2` e `+ 3 4` → 7 (o segundo item)
#[test]
fn dois_items_separados() {
    let src = "+ 1 2\n+ 3 4";
    let arities = prelude_arities();
    let (raw, ty) = eval_src_arity(src, arities);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // O REPL/driver avalia o último item → + 3 4 = 7
    assert_eq!(untag_smi(raw), 7, "último item + 3 4 deve ser 7");
}
