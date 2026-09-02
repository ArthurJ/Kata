use super::*;
use std::io::Write;

fn create_temp_file(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

#[test]
fn load_simple_module() {
    let tmp = tempfile::tempdir().unwrap();
    create_temp_file(tmp.path(), "simple.kata", "42");

    let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
    let resolved = loader.load(&["simple".into()], tmp.path()).unwrap();

    // 42 é a entry expr — sem declarações do usuário. Mas load_path
    // agora injeta stdlib pré-carregada (merge_two), então signatures
    // contém as assinaturas do prelude (Int, Float, +, *, etc.).
    // Verificamos que o módulo carrega sem erro e tem prelude.
    assert!(!resolved.signatures.is_empty());
    assert!(resolved.signatures.iter().any(|s| s.name == "+"));
}

#[test]
fn load_with_subdirectory() {
    let tmp = tempfile::tempdir().unwrap();
    create_temp_file(
        tmp.path(),
        "util/math.kata",
        "+ :: Int Int => Int @ffi(\"kata_rt_bi_add\")",
    );

    let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
    let resolved = loader
        .load(&["util".into(), "math".into()], tmp.path())
        .unwrap();
    // A assinatura do usuário (+) está entre as signatures (junto com prelude).
    assert!(resolved.signatures.iter().any(|s| s.name == "+"));
}

#[test]
fn cache_returns_same_module() {
    let tmp = tempfile::tempdir().unwrap();
    create_temp_file(tmp.path(), "cached.kata", "42");

    let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
    let first = loader.load(&["cached".into()], tmp.path()).unwrap();
    let second = loader.load(&["cached".into()], tmp.path()).unwrap();
    // Arc clones — mesmo ponteiro
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn not_found_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
    let err = loader
        .load(&["nonexistent".into()], tmp.path())
        .unwrap_err();
    assert!(matches!(err, LoadError::NotFound { .. }));
}

#[test]
fn circular_import_detected() {
    let tmp = tempfile::tempdir().unwrap();
    // Simula ciclo: a.kata importa b, b.kata importa a.
    // Mas para detectar o ciclo, precisamos que o loader
    // esteja carregando a quando tenta carregar a novamente.
    // Como o loader não processa imports internamente (isso é
    // responsabilidade do resolution), o teste manual insere
    // path em loading e tenta carregar.
    let path = create_temp_file(tmp.path(), "circular.kata", "42");
    let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
    // Simula que o arquivo já está sendo carregado
    loader.loading.insert(path.clone());
    let err = loader.load_path(&path).unwrap_err();
    assert!(matches!(err, LoadError::CircularImport { .. }));
}

#[test]
fn search_path_fallback() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    // Arquivo só existe em tmp2
    create_temp_file(tmp2.path(), "found.kata", "42");

    let mut loader = ModuleLoader::new(vec![tmp1.path().to_path_buf(), tmp2.path().to_path_buf()]);
    let resolved = loader.load(&["found".into()], tmp1.path()).unwrap();
    // load_path injeta stdlib pré-carregada — signatures contém prelude.
    assert!(!resolved.signatures.is_empty());
    assert!(resolved.signatures.iter().any(|s| s.name == "+"));
}

#[test]
fn load_imports_returns_imported_modules() {
    let tmp = tempfile::tempdir().unwrap();
    // Módulo exportador com export explícito
    create_temp_file(
        tmp.path(),
        "math_utils.kata",
        "dobrar :: Int => Int\nlambda x: + x x\n\ntriplicar :: Int => Int\nlambda x: * x 3\n\nexport dobrar triplicar",
    );
    // Módulo importador
    create_temp_file(
        tmp.path(),
        "main.kata",
        "import math_utils\nimport math_utils.(triplicar)\n\n42",
    );

    let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
    let source = std::fs::read_to_string(tmp.path().join("main.kata")).unwrap();
    let tokens = lex(&source).unwrap();
    let module = parse(tokens).unwrap();

    let imports = loader.load_imports(&module, tmp.path()).unwrap();
    // Dois imports (do mesmo módulo — cache retorna Arc igual)
    assert_eq!(imports.len(), 2);

    // Primeiro: WholeModule { prefix: "math_utils" }
    assert!(matches!(
        &imports[0].import_kind,
        ImportKind::WholeModule { prefix } if prefix == "math_utils"
    ));

    // Segundo: Selective { items: [ImportItem { name: "triplicar", alias: None }] }
    assert!(matches!(
        &imports[1].import_kind,
        ImportKind::Selective { items } if items.len() == 1 && items[0].name == "triplicar" && items[0].alias.is_none()
    ));

    // O módulo exportador tem export → só dobrar e triplicar visíveis.
    // Ambas são signatures (sigs com corpo Kata = functions também).
    let resolved = &imports[0].resolved;
    let sig_names: Vec<&str> = resolved
        .signatures
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert!(sig_names.contains(&"dobrar"));
    assert!(sig_names.contains(&"triplicar"));
}

#[test]
fn filter_exports_no_export_decl_is_open() {
    let tmp = tempfile::tempdir().unwrap();
    // Módulo sem export — tudo é exportado
    create_temp_file(
        tmp.path(),
        "open.kata",
        "foo :: Int => Int @ffi(\"kata_rt_bi_add\")\nbar :: Int => Int @ffi(\"kata_rt_bi_sub\")",
    );

    let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
    let resolved = loader.load(&["open".into()], tmp.path()).unwrap();
    // Sem export decl → ambas signatures visíveis (junto com prelude).
    assert!(resolved.signatures.iter().any(|s| s.name == "foo"));
    assert!(resolved.signatures.iter().any(|s| s.name == "bar"));
}

#[test]
fn filter_exports_with_export_decl_filters() {
    let tmp = tempfile::tempdir().unwrap();
    // Módulo com export — só `public_fn` é visível
    create_temp_file(
        tmp.path(),
        "filtered.kata",
        "public_fn :: Int => Int @ffi(\"kata_rt_bi_add\")\nprivate_fn :: Int => Int @ffi(\"kata_rt_bi_sub\")\n\nexport public_fn",
    );

    let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
    // `load` retorna não-filtrado agora. Precisamos chamar filter_exports
    // explicitamente para obter a versão filtrada.
    let resolved = loader.load(&["filtered".into()], tmp.path()).unwrap();
    // Re-lex/parse para obter o AST do módulo (filter_exports precisa do Module)
    let source = std::fs::read_to_string(tmp.path().join("filtered.kata")).unwrap();
    let tokens = lex(&source).unwrap();
    let module = parse(tokens).unwrap();
    let filtered = filter_exports((*resolved).clone(), &module);
    // Só public_fn visível (private_fn filtrada pelo export).
    // Prelude também está presente mas não tem public_fn/private_fn.
    assert!(filtered.signatures.iter().any(|s| s.name == "public_fn"));
    assert!(!filtered.signatures.iter().any(|s| s.name == "private_fn"));
}

#[test]
fn filter_exports_internal_signatures_from_function_body() {
    let tmp = tempfile::tempdir().unwrap();
    // Módulo com função exportada que chama função interna não-exportada.
    // `exported_fn` chama `internal_fn` no seu corpo lambda.
    // `internal_fn` é @ffi (signature sem corpo).
    create_temp_file(
        tmp.path(),
        "internal.kata",
        "internal_fn :: Int => Int @ffi(\"kata_rt_bi_add\")\n\
             exported_fn :: Int => Int\n\
             lambda x: internal_fn x\n\
             \n\
             export exported_fn",
    );

    let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
    let resolved = loader.load(&["internal".into()], tmp.path()).unwrap();
    let source = std::fs::read_to_string(tmp.path().join("internal.kata")).unwrap();
    let tokens = lex(&source).unwrap();
    let module = parse(tokens).unwrap();
    let filtered = filter_exports((*resolved).clone(), &module);

    // exported_fn está nas signatures exportadas.
    assert!(
        filtered.signatures.iter().any(|s| s.name == "exported_fn"),
        "exported_fn deve estar em signatures"
    );
    // internal_fn NÃO está em signatures (não exportada).
    assert!(
        !filtered.signatures.iter().any(|s| s.name == "internal_fn"),
        "internal_fn NÃO deve estar em signatures"
    );
    // internal_fn ESTÁ em internal_signatures (dependência do corpo).
    assert!(
        filtered
            .internal_signatures
            .iter()
            .any(|s| s.name == "internal_fn"),
        "internal_fn deve estar em internal_signatures"
    );
}

#[test]
fn filter_exports_internal_signatures_transitive() {
    let tmp = tempfile::tempdir().unwrap();
    // Cadeia transitiva: exported_fn → mid_fn → base_fn.
    // Apenas exported_fn é exportada; mid_fn e base_fn são internas.
    create_temp_file(
        tmp.path(),
        "transitive.kata",
        "base_fn :: Int => Int @ffi(\"kata_rt_bi_add\")\n\
             mid_fn :: Int => Int\n\
             lambda x: base_fn x\n\
             \n\
             exported_fn :: Int => Int\n\
             lambda x: mid_fn x\n\
             \n\
             export exported_fn",
    );

    let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
    let resolved = loader.load(&["transitive".into()], tmp.path()).unwrap();
    let source = std::fs::read_to_string(tmp.path().join("transitive.kata")).unwrap();
    let tokens = lex(&source).unwrap();
    let module = parse(tokens).unwrap();
    let filtered = filter_exports((*resolved).clone(), &module);

    assert!(filtered.signatures.iter().any(|s| s.name == "exported_fn"));
    assert!(!filtered.signatures.iter().any(|s| s.name == "mid_fn"));
    assert!(!filtered.signatures.iter().any(|s| s.name == "base_fn"));
    assert!(
        filtered
            .internal_signatures
            .iter()
            .any(|s| s.name == "mid_fn"),
        "mid_fn deve estar em internal_signatures (chamada por exported_fn)"
    );
    assert!(
        filtered
            .internal_signatures
            .iter()
            .any(|s| s.name == "base_fn"),
        "base_fn deve estar em internal_signatures (chamada transitivamente por mid_fn)"
    );
}
