//! Testes E2E de dict dispatch — actions com params nomeados.
//!
//! Pipeline completo: lex -> parse -> resolve -> infer -> optimize -> codegen -> JIT.
//! Valida que `f!{"a": 1, "b": 2}` despacha para actions com params nomeados
//! (sintaxe `!{}` = dict nomeado) e que `f!({...})` passa dict como valor
//! posicional (sintaxe `!(` = tupla posicional).
//!
//! Funções puras são exclusivamente posicionais — `f{"k": v}` sem `!` passa
//! o dict como valor posicional, não como args nomeados.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
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
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Verifica que o pipeline falha em algum estágio (lex, parse, resolve, ou infer).
fn infer_fails(src: &str) -> bool {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let module = match parse(tokens) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let prelude = match load_stdlib_for_tests() {
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

// ── Dict dispatch em actions com params nomeados (`!{}`) ────────

/// `action soma (a::Int, b::Int) => Int` + `soma!{"a": 3 "b": 4}` → 7.
/// A sintaxe `!{` abre dict nomeado — o prólogo reordena chaves para params.
#[test]
fn dict_dispatch_simples() {
    let src = "action soma (a::Int, b::Int) => Int\n    + a b\nsoma!{\"a\": 3 \"b\": 4}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 7, "soma com dict nomeado deve ser 7");
}

/// Ordem das chaves no dict não importa — reordena para params.
/// `soma!{"b": 4 "a": 3}` deve retornar 7.
#[test]
fn dict_dispatch_ordem_invertida() {
    let src = "action soma (a::Int, b::Int) => Int\n    + a b\nsoma!{\"b\": 4 \"a\": 3}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        7,
        "soma com dict ordem invertida deve ser 7"
    );
}

/// Action com 3 params nomeados.
/// `action sub (x::Int, y::Int, z::Int) => Int` + `sub!{"z": 1 "x": 10 "y": 3}` → 6.
#[test]
fn dict_dispatch_tres_params() {
    let src = "action sub (x::Int, y::Int, z::Int) => Int\n    - (- x y) z\nsub!{\"z\": 1 \"x\": 10 \"y\": 3}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 6, "sub com 3 params deve ser 6");
}

// ── Teste negativo: função pura não aceita dict dispatch ────────

/// `dobro :: Int => Int` + `dobro{"x": 21}` deve falhar.
/// Funções puras são exclusivamente posicionais — `f{"k": v}` sem `!`
/// passa o dict como valor posicional. Como `dobro` espera `Int` e
/// recebe `Dict`, o typeck deve rejeitar com erro de tipo.
#[test]
fn dict_dispatch_funcao_pura_sem_bang() {
    let src = "dobro :: Int => Int\nlambda x: * x 2\ndobro{\"x\": 21}";
    assert!(
        infer_fails(src),
        "função pura não aceita dict como args nomeados — deve falhar no typeck"
    );
}

// ── Dict como valor posicional em função (`f ({...})`) ──────────

/// `mostra :: Dict::(Text, Int) => Text` + `mostra ({"chave": 42})`.
/// O dict é passado como valor posicional dentro de tupla de 1 elemento.
#[test]
fn dict_como_valor_posicional_em_funcao() {
    let src = "mostra :: Dict::(Text, Int) => Text\nlambda d: show d\nmostra ({\"chave\": 42})";
    let (raw, ty) = eval_src(src);
    let _ = raw;
    assert_eq!(ty, Ty::Prim(PrimTy::Text), "mostra deve retornar Text");
}

// ── Dict como valor vs args nomeados em action ──────────────────

/// `config!({"timeout": 30})` — dict como valor posicional dentro de `!(`.
/// A action recebe Dict como tipo de parâmetro.
#[test]
fn dict_como_valor_posicional_em_action() {
    let src = "action config (opts::Dict::(Text, Int)) => Int\n    + (len opts) 0\nconfig!({\"timeout\": 30})";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 1, "dict com 1 entrada → len = 1");
}

/// `config!{\"timeout\": 30}` com action que recebe Dict como tipo
/// deve falhar — `!{` é dict nomeado, e a action não tem param "timeout".
/// A action tem param `opts::Dict::(Text, Int)`, não `timeout::Int`.
#[test]
fn dict_nomeado_em_action_que_recebe_dict_como_tipo() {
    let src = "action config (opts::Dict::(Text, Int)) => Int\n    + (len opts) 0\nconfig!{\"timeout\": 30}";
    assert!(
        infer_fails(src),
        "config!{{...}} é dict nomeado — action não tem param 'timeout', deve falhar"
    );
}

// ── Default args via dict-template ────────────────────────────

/// Action com defaults, chamada nomeada omitindo arg com default.
/// `action act{msg::Text: _, dft::Int: 5}` + `act!{"msg": "hi"}` → dft=5 (default).
#[test]
fn default_args_nomeada_omitindo_default() {
    let src =
        "action act{msg::Text: _, dft::Int: 5} => Int\n    + dft (len msg)\nact!{\"msg\": \"hi\"}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // dft=5 (default) + len("hi")=2 → 7
    assert_eq!(
        untag_smi(raw),
        7,
        "dft deve usar default 5: 5 + len(\"hi\") = 7"
    );
}

/// Action com defaults, chamada posicional omitindo arg com default.
/// `act!("hi")` → msg="hi", dft=5 (default).
#[test]
fn default_args_posicional_omitindo_default() {
    let src = "action act{msg::Text: _, dft::Int: 5} => Int\n    + dft (len msg)\nact!(\"hi\")";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // dft=5 (default) + len("hi")=2 → 7
    assert_eq!(
        untag_smi(raw),
        7,
        "dft deve usar default 5: 5 + len(\"hi\") = 7"
    );
}

/// Action com defaults, chamada nomeada sobrescrevendo default.
/// `act!{"msg": "hi", "dft": 10}` → dft=10 (sobrescrito).
#[test]
fn default_args_nomeada_sobrescrevendo_default() {
    let src = "action act{msg::Text: _, dft::Int: 5} => Int\n    + dft (len msg)\nact!{\"msg\": \"hi\" \"dft\": 10}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // dft=10 (sobrescrito) + len("hi")=2 → 12
    assert_eq!(
        untag_smi(raw),
        12,
        "dft sobrescrito para 10: 10 + len(\"hi\") = 12"
    );
}

/// Action sem defaults (sintaxe `(x::Int)`) continua funcionando.
/// `action soma (a::Int, b::Int) => Int` + `soma!{"a": 3 "b": 4}` → 7.
#[test]
fn default_args_action_sem_defaults_continua_funcionando() {
    let src = "action soma (a::Int, b::Int) => Int\n    + a b\nsoma!{\"a\": 3 \"b\": 4}";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        7,
        "action sem defaults deve funcionar como antes"
    );
}

/// Negativo: action com defaults, chamada nomeada omitindo arg obrigatório (`_`).
/// `act!{"dft": 3}` sem `msg` (que é `_` = obrigatório) → erro.
#[test]
fn default_args_omitindo_obrigatorio_falha() {
    let src = "action act{msg::Text: _, dft::Int: 5} => Int\n    + dft (len msg)\nact!{\"dft\": 3}";
    assert!(
        infer_fails(src),
        "act!{{\"dft\": 3}} sem msg (obrigatório) deve falhar"
    );
}

/// Negativo: action com defaults, chamada posicional omitindo arg obrigatório.
/// `act!()` sem args → msg é obrigatório, deve falhar.
#[test]
fn default_args_posicional_sem_obrigatorio_falha() {
    let src = "action act{msg::Text: _, dft::Int: 5} => Int\n    + dft (len msg)\nact!()";
    assert!(infer_fails(src), "act!() sem msg (obrigatório) deve falhar");
}
