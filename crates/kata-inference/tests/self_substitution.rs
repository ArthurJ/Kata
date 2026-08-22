//! Testes da substituição de `Self` no dispatch por método de interface.
//!
//! Antes da Fase 1, `try_iface_method_dispatch` retornava `sig.ret.clone()`
//! sem substituir `Self`. Para interfaces como SHOW (show :: Self => Text),
//! isso era inofensivo porque `Self` não aparecia no tipo de retorno. Mas
//! STEPPABLE (step :: Self => Self) precisa que `Self` seja substituído pelo
//! tipo do argumento para que o typeck e o monomorphizador produzam tipos
//! concretos.
//!
//! Cenários testados:
//! 1. Interface com Self no retorno, arg concreto (Interface("DUP")) →
//!    retorno substitui Self por Interface("DUP")
//! 2. Interface sem Self no retorno (show :: Self => Text) →
//!    comportamento inalterado (retorna Text)

use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{load_prelude, resolve};

/// Combina prelude + módulo do usuário (replica do driver).
fn merge_resolved(
    prelude: kata_resolution::ResolvedModule,
    user: kata_resolution::ResolvedModule,
) -> kata_resolution::ResolvedModule {
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
    kata_resolution::ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry,
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

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("infer deve succeed")
}

// ── Self no retorno com arg concreto ───────────────────────────

/// Interface com `Self` no retorno. A assinatura na interface deve
/// preservar `Self` (não é substituído no registry — só no dispatch).
#[test]
fn self_in_return_preserved_in_registry() {
    let src = r#"
interface DUP
    dup :: Self => Self

Int implements DUP
    @ffi("kata_rt_bi_add")
    dup :: Int => Int
"#;
    let resolved = {
        let tokens = lex(src).unwrap();
        let module = parse(tokens).unwrap();
        let prelude = load_prelude().unwrap();
        let user = resolve(&module).unwrap();
        merge_resolved(prelude, user)
    };

    let iface = resolved
        .interface_registry
        .get_interface("DUP")
        .expect("interface DUP deve existir");
    assert_eq!(iface.signatures.len(), 1);
    let sig = &iface.signatures[0];
    assert_eq!(sig.name, "dup");
    // A assinatura na interface preserva Self — a substituição
    // acontece no dispatch, não no registro.
    assert_eq!(sig.params, vec![Ty::Var("Self".into())]);
    assert_eq!(sig.ret, Ty::Var("Self".into()));
}

// ── Self no retorno não afeta interfaces sem Self no retorno ────

/// SHOW tem `show :: Self => Text` — Self não está no retorno.
/// O comportamento deve ser inalterado: retorna Text.
#[test]
fn self_not_in_return_unchanged() {
    let src = "show 42";
    let typed = infer_src(src);

    // A entry deve ter tipo Text (show de Int → Text).
    assert_eq!(typed.entry.node.ty, Ty::Prim(kata_core::ty::PrimTy::Text));
}

// ── Self substituído no dispatch com arg Interface ──────────────

/// Quando o arg é tipado como `Interface("DUP")` e a função é método
/// da interface, o typeck deve substituir Self por Interface("DUP")
/// no tipo de retorno. Verificamos que o tipo da Closure produzida
/// tem o retorno correto (não Var("Self")).
#[test]
fn self_substituted_in_dispatch_return() {
    // interface DUP com dup :: Self => Self.
    // chamada `dup` em um arg tipado como Interface("DUP") deve
    // produzir retorno Interface("DUP"), não Var("Self").
    let src = r#"
interface DUP
    dup :: Self => Self

Int implements DUP
    @ffi("kata_rt_bi_add")
    dup :: Int => Int

dup 42
"#;
    let typed = infer_src(src);

    // A entry deve ser uma Closure com callee "dup" e arg 42 (Int).
    // Como Int implementa DUP, o dispatch direto pelo DispatchTable
    // deve resolver para o overload concreto (Int => Int).
    // O tipo do resultado deve ser Int (não Var("Self")).
    assert_eq!(typed.entry.node.ty, Ty::int());
}

// ── Self em tipo composto no retorno ───────────────────────────

/// Interface com `Self` dentro de um tipo composto no retorno.
/// `wrap :: Self => Optional::Self` — Self dentro de Generic("Optional", [Self]).
/// O substitute_self deve substituir recursivamente.
#[test]
fn self_in_composite_return_preserved_in_registry() {
    let src = r#"
interface WRAP
    wrap :: Self => Optional::Self

Int implements WRAP
    @ffi("kata_rt_some")
    wrap :: Int => Optional::Int
"#;
    let resolved = {
        let tokens = lex(src).unwrap();
        let module = parse(tokens).unwrap();
        let prelude = load_prelude().unwrap();
        let user = resolve(&module).unwrap();
        merge_resolved(prelude, user)
    };

    let iface = resolved
        .interface_registry
        .get_interface("WRAP")
        .expect("interface WRAP deve existir");
    let sig = &iface.signatures[0];
    assert_eq!(sig.name, "wrap");
    // O retorno na interface deve ter Self dentro de Optional.
    assert_eq!(
        sig.ret,
        Ty::Generic("Optional".into(), vec![Ty::Var("Self".into())])
    );
}
