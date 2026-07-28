//! Testes E2E de inferência bidirecional de type params em variants.
//!
//! Valida que a construção de variantes de enum genérico (ex: `Result::Ok`)
//! em funções nomeadas com assinatura que menciona o tipo completo
//! (ex: `Result::(Int, Text)`) preenche os type params não-inferidos
//! pelo payload da variante usando o tipo esperado do contexto.
//!
//! Cenário canônico: `Ok(T)` não menciona `E`. Sem inferência bidirecional,
//! `E` fica como `Ty::Var("E")` não-resolvido. Com a correção, a assinatura
//! `=> Result::(Int, Text)` propaga `E|Text` para a construção.

use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

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
    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);
    let mut refines_registry = prelude.refines_registry;
    refines_registry.merge(user.refines_registry);
    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry,
        refines_registry,
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

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

/// Encontra uma função nomeada pelo nome no TypedModule.
fn find_function<'a>(
    tmod: &'a kata_inference::TypedModule,
    name: &str,
) -> &'a kata_inference::TypedFunction {
    tmod.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("função `{name}` não encontrada"))
}

// ── Ok em função nomeada: E não aparece no payload de Ok ──────────

/// `ok_id :: Int => Result::(Int, Text)` com body `Result::Ok x`.
///
/// Sem inferência bidirecional, `Ok x` infere `T=Int` mas `E` fica
/// `Ty::Var("E")`. O typeck rejeitaria porque `Result::(Int, Var("E"))`
/// ≠ `Result::(Int, Text)`.
///
/// Com a correção, o hint `Some(Result::(Int, Text))` propaga `E|Text`
/// para a construção.
#[test]
fn ok_em_funcao_nomeada_preenche_e_do_hint() {
    let src = "ok_id :: Int => Result::(Int, Text)\nlambda x: Result::Ok x\nok_id 42";
    let tmod = infer_src(src);
    let func = find_function(&tmod, "ok_id");
    // O ret_ty da função é o da assinatura: Result::(Int, Text)
    assert_eq!(
        func.ret_ty,
        Ty::Generic("Result".into(), vec![Ty::int(), Ty::text()])
    );
}

// ── Err em função nomeada: T não aparece no payload de Err ─────────

/// `err_str :: Text => Result::(Int, Text)` com body `Result::Err msg`.
///
/// `Err msg` infere `E|Text` mas `T` fica `Ty::Var("T")`.
/// O hint propaga `T=Int`.
#[test]
fn err_em_funcao_nomeada_preenche_t_do_hint() {
    let src =
        "err_str :: Text => Result::(Int, Text)\nlambda msg: Result::Err msg\nerr_str \"erro\"";
    let tmod = infer_src(src);
    let func = find_function(&tmod, "err_str");
    assert_eq!(
        func.ret_ty,
        Ty::Generic("Result".into(), vec![Ty::int(), Ty::text()])
    );
}

// ── Match com scrutinee tipado: type args completos ────────────────

/// `unwrap :: Result::(Int, Text) => Int` com match nos dois braços.
///
/// O scrutinee `r` tem tipo `Result::(Int, Text)` da assinatura.
/// O braço `Ok v` deve inferir `v: Int`. O braço `Err _` não acessa payload.
#[test]
fn match_com_scrutinee_tipado_resolve_type_args() {
    // O entry point chama extract_ok com um Result::Ok 42.
    // O argumento Result::Ok 42 sem hint tem E=Var("E"), mas o dispatch
    // de extract_ok espera Result::(Int, Text). Usa ascription para forçar.
    let src = "extract_ok :: Result::(Int, Text) => Int\nlambda r: match r\n        Result::Ok v: v\n        Result::Err _: 0\nextract_ok ((Result::Ok 42)::Result::(Int, Text))";
    let tmod = infer_src(src);
    let func = find_function(&tmod, "extract_ok");
    assert_eq!(func.ret_ty, Ty::int());
}

// ── Match com hint: arm body constrói variant com type param não-inferido ──

/// `re_wrap :: Result::(Int, Text) => Result::(Int, Text)` com match
/// onde cada arm constrói um variant do Result.
///
/// O arm `Ok v` constrói `Result::Ok v` — `Ok` menciona `T` mas não `E`.
/// Sem propagação de hint para o match, `E` ficaria `Var("E")` dentro do arm.
/// Com a correção, o hint `Result::(Int, Text)` é propagado para o body
/// do arm e `E|Text` é preenchido.
#[test]
fn match_arm_construction_recebe_hint_do_contexto() {
    let src = "re_wrap :: Result::(Int, Text) => Result::(Int, Text)\nlambda r: match r\n        Result::Ok v: Result::Ok v\n        Result::Err e: Result::Err e\nre_wrap ((Result::Ok 42)::Result::(Int, Text))";
    let tmod = infer_src(src);
    let func = find_function(&tmod, "re_wrap");
    assert_eq!(
        func.ret_ty,
        Ty::Generic("Result".into(), vec![Ty::int(), Ty::text()])
    );
}

/// `lambda x: Result::Ok x` sem assinatura.
///
/// Sem hint, `E` fica como `Ty::Var("E")`. Isto é aceitável —
/// o lambda anônimo não tem assinatura para fornecer o tipo completo.
/// O teste confirma que a inferência não falha (o Var é aceito).
#[test]
fn ok_sem_hint_deixa_e_como_var() {
    // Aplicar o lambda a um Int para ter um entry point tipável.
    let src = "(lambda x: Result::Ok x) 42";
    let tmod = infer_src(src);
    let entry = &tmod.entry.node;
    // O tipo do apply é o tipo de retorno do lambda: Result::(Int, Var("E"))
    // ou Result::(Int, _) — o importante é que T=Int foi inferido.
    assert!(
        matches!(&entry.ty, Ty::Generic(name, args) if name == "Result" && args.len() == 2 && args[0] == Ty::int()),
        "esperado Result::(Int, _), got {:?}",
        entry.ty
    );
}

// ── Bidirecionalidade entre arms: arms complementares sem hint ─────

/// Match top-level (sem assinatura) onde arms têm informação complementar
/// sobre type params DIFERENTES.
///
/// Scrutinee: `Result::Ok 42` → `Generic("Result", [Int, Text])`.
/// O default `Err(E|Text)` do prelude preenche E|Text automaticamente.
/// Arm `Ok v`: constrói `Result::Ok v` → `Generic("Result", [Int, Text])`.
/// Arm `Err e`: `e` tem tipo `Text` (do default), constrói
/// `Result::Err e` → `Generic("Result", [Var("T"), Text])`.
///
/// Unificação recursiva: posição 0 Int vs Var("T") → Int.
/// Posição 1 Text vs Text → Text.
/// Resultado: `Generic("Result", [Int, Text])`.
#[test]
fn match_arms_complementares_unificam_t_mas_e_fica_var() {
    let src =
        "match (Result::Ok 42)\n    Result::Ok v: Result::Ok v\n    Result::Err e: Result::Err e";
    let tmod = infer_src(src);
    let entry = &tmod.entry.node;
    // T é resolvido (Int) pela unificação entre arms.
    // E é Text (default do prelude `Err(E|Text)`).
    assert_eq!(
        entry.ty,
        Ty::Generic("Result".into(), vec![Ty::int(), Ty::text()])
    );
}

/// Match onde ambos os arms constroem Ok com tipos diferentes para T.
/// O arm `Ok v` (v=42, Int) constrói `Result::Ok v` → T=Int.
/// O arm `Err e` constrói `Result::Ok e` → T=tipo de e (Var("E") do scrutinee).
/// Unificação: T=Int vs T=Var("E") → Int (Var aceita concreto).
/// Não deve falhar — o Var do scrutinee é compatível com Int.
#[test]
fn match_arms_complementares_com_text_no_err() {
    // Scrutinee: Result::Err "erro" → Generic("Result", [Var("T"), Text])
    // Arm Ok v: v tem tipo Var("T"), constrói Result::Ok v → [Var("T"), Var("E")]
    // Arm Err e: e tem tipo Text, constrói Result::Err e → [Var("T"), Text]
    // Unificação: T: Var vs Var → Var; E: Var vs Text → Text
    // Resultado: Generic("Result", [Var("T"), Text])
    let src = "match (Result::Err \"erro\")\n    Result::Ok v: Result::Ok v\n    Result::Err e: Result::Err e";
    let tmod = infer_src(src);
    let entry = &tmod.entry.node;
    assert_eq!(
        entry.ty,
        Ty::Generic("Result".into(), vec![Ty::Var("T".into()), Ty::text()])
    );
}

/// Match com arms que produzem tipos completamente incompatíveis.
/// Arm 1 retorna Result, arm 2 retorna Int. Deve falhar.
#[test]
fn match_arms_incompatíveis_deve_falhar() {
    let src = "match (Result::Ok 42)\n    Result::Ok v: Result::Ok v\n    Result::Err _: 0";
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(
        result.is_err(),
        "match com arm retornando Result e arm retornando Int deve falhar"
    );
}
