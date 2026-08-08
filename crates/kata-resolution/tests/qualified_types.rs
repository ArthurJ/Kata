//! Testes E2E para tipos qualificados (module.Type) e origin em TypeBinding.
//!
//! Verifica que:
//! 1. Tipos de módulos importados são copiados para o type_env do importador
//! 2. `module.Type` resolve corretamente via origin
//! 3. Ambiguidade é detectada quando dois módulos definem o mesmo tipo
//! 4. Local shadowa imports sem ambiguidade

use kata_core::{Ty, TypeEnv};
use kata_resolution::{ResolvedModule, merge_two, resolve_with_origin};

// Helper: cria um ResolvedModule mínimo com um tipo definido.
fn make_module_with_type(name: &str, ty: Ty, origin: &str) -> ResolvedModule {
    let mut type_env = TypeEnv::new();
    type_env.define(name, ty, origin);
    ResolvedModule {
        type_env,
        signatures: Vec::new(),
        enum_registry: kata_core::EnumRegistry::new(),
        struct_registry: kata_core::StructRegistry::new(),
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry: kata_core::InterfaceRegistry::new(),
        refines_registry: kata_core::RefinesRegistry::new(),
        functions: Vec::new(),
        actions: Vec::new(),
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

#[test]
fn typebinding_origin_e_lookup_binding() {
    let mut env = TypeEnv::new();
    env.define("Pessoa", Ty::Struct("Pessoa".into()), "my_module");

    // lookup retorna apenas Ty
    assert_eq!(env.lookup("Pessoa"), Some(&Ty::Struct("Pessoa".into())));

    // lookup_binding retorna TypeBinding com origin
    let binding = env.lookup_binding("Pessoa").expect("binding deve existir");
    assert_eq!(binding.ty, Ty::Struct("Pessoa".into()));
    assert_eq!(binding.origin, "my_module");
}

#[test]
fn merge_bindings_from_marcas_ambiguidade() {
    let mut env_a = TypeEnv::new();
    env_a.define("Result", Ty::Struct("Result".into()), "module_a");

    let mut env_b = TypeEnv::new();
    env_b.define("Result", Ty::Struct("Result".into()), "module_b");

    // Merge: mesmo nome, origins diferentes → ambíguo
    env_a.merge_bindings_from(&mut env_b);

    assert!(env_a.is_ambiguous("Result"));
    // Tipos não-ambíguos não são marcados
    assert!(!env_a.is_ambiguous("NonExistent"));
}

#[test]
fn merge_bindings_from_mesma_origin_nao_e_ambiguo() {
    let mut env_a = TypeEnv::new();
    env_a.define("Result", Ty::Struct("Result".into()), "module_a");

    let mut env_b = TypeEnv::new();
    env_b.define("Result", Ty::Struct("Result".into()), "module_a");

    // Mesmo nome, mesma origin → não é ambíguo (mesmo módulo)
    env_a.merge_bindings_from(&mut env_b);

    assert!(!env_a.is_ambiguous("Result"));
}

#[test]
fn local_shadowa_import_sem_ambiguidade() {
    let mut env = TypeEnv::new();
    // Import de module_a
    env.define("Result", Ty::Struct("Result".into()), "module_a");
    // Local define o mesmo nome
    env.define("Result", Ty::Struct("Result".into()), "__local__");

    // Local shadowa — lookup retorna o local
    let binding = env.lookup_binding("Result").expect("binding");
    assert_eq!(binding.origin, "__local__");
    // Não é ambíguo porque local tem prioridade
    // (is_ambiguous verifica o set, que só é populado por merge_bindings_from)
}

#[test]
fn resolve_with_origin_popula_origin_nos_bindings() {
    use kata_lexer::lex;
    use kata_parser::parse;

    let source = "data Pessoa (nome::Text idade::Int)\n42";
    let tokens = lex(source).unwrap();
    let module = parse(tokens).unwrap();
    let resolved = resolve_with_origin(&module, "my_module").expect("resolve");

    let binding = resolved
        .type_env
        .lookup_binding("Pessoa")
        .expect("Pessoa deve estar no type_env");
    assert_eq!(binding.ty, Ty::Struct("Pessoa".into()));
    assert_eq!(binding.origin, "my_module");
}

#[test]
fn resolve_com_origin_default_usa_local() {
    use kata_lexer::lex;
    use kata_parser::parse;
    use kata_resolution::resolve;

    let source = "data Pessoa (nome::Text)\n42";
    let tokens = lex(source).unwrap();
    let module = parse(tokens).unwrap();
    let resolved = resolve(&module).expect("resolve");

    let binding = resolved
        .type_env
        .lookup_binding("Pessoa")
        .expect("Pessoa deve estar no type_env");
    assert_eq!(binding.origin, "__local__");
}

#[test]
fn type_expr_qualified_parse_e_resolve() {
    use kata_lexer::lex;
    use kata_parser::parse;

    // `mock_math.Result` em posição de tipo
    let source = "x :: mock_math.Result => mock_math.Result\nlambda x: x\n42";
    let tokens = lex(source).unwrap();
    let module = parse(tokens).unwrap();
    let resolved = resolve_with_origin(&module, "main").expect("resolve");

    // O signature deve ter sido parseada com TypeExpr::Qualified
    // e resolvida para Ty::Struct("Result") via fallback
    let sig = resolved
        .signatures
        .iter()
        .find(|s| s.name == "x")
        .expect("signature x deve existir");

    // return_type já é Ty resolvida — deve ser Ty::Struct("Result")
    // (fallback de Qualified quando o módulo não está importado)
    assert_eq!(sig.return_type, Ty::Struct("Result".into()));
}

#[test]
fn merge_two_preserva_origins() {
    let prelude = make_module_with_type("Int", Ty::int(), "core");
    let user = make_module_with_type("Pessoa", Ty::Struct("Pessoa".into()), "my_module");

    let merged = merge_two(prelude, user);

    // Int veio do core
    let int_binding = merged.type_env.lookup_binding("Int").expect("Int");
    assert_eq!(int_binding.origin, "core");

    // Pessoa veio do my_module
    let pessoa_binding = merged.type_env.lookup_binding("Pessoa").expect("Pessoa");
    assert_eq!(pessoa_binding.origin, "my_module");
}
