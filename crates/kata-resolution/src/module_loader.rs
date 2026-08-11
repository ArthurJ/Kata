//! Module Loader — carrega módulos do filesystem com cache e cycle detection.
//!
//! Infraestrutura para `import modulo.submodulo`.
//! O loader resolve paths, parsea, resolve, filtra exports, e cacheia o resultado.
//! Ciclos de import são detectados via HashSet de paths em loading.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kata_ast::{Item, Module};
use kata_lexer::lex;
use kata_parser::parse;

use crate::{ResolvedModule, merge_two, resolve_with_origin};

/// Erro de carregamento de módulo.
#[derive(Debug, Clone)]
pub enum LoadError {
    /// Arquivo não encontrado em nenhum search path.
    NotFound { path: String },
    /// Erro de lex (preserva FrontendError estruturado com Span).
    Lex(kata_diagnostics::FrontendError),
    /// Erro de parse (preserva FrontendError estruturado com Span).
    Parse(kata_diagnostics::FrontendError),
    /// Erro de resolution (preserva Vec<ResolveError> estruturado).
    Resolve(Vec<crate::ResolveError>),
    /// Ciclo de import detectado.
    CircularImport { path: String },
    /// Erro de I/O.
    Io(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotFound { path } => write!(f, "módulo não encontrado: `{path}`"),
            LoadError::Lex(e) => write!(f, "erro léxico ao carregar módulo: {e}"),
            LoadError::Parse(e) => write!(f, "erro de parse ao carregar módulo: {e}"),
            LoadError::Resolve(errors) => {
                write!(
                    f,
                    "erro de resolução ao carregar módulo: {}",
                    crate::format_resolve_errors(errors)
                )
            }
            LoadError::CircularImport { path } => {
                write!(f, "ciclo de import detectado: `{path}`")
            }
            LoadError::Io(msg) => write!(f, "erro de I/O ao carregar módulo: {msg}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Como um módulo foi importado.
#[derive(Debug, Clone)]
pub enum ImportKind {
    /// `import mod` — módulo inteiro, acesso via `mod.fn`.
    /// O prefixo é o último componente do path.
    WholeModule { prefix: String },
    /// `import mod as alias` — módulo inteiro, acesso via `alias.fn`.
    WholeModuleAliased { alias: String },
    /// `import MOD.(item1 item2)` ou `import MOD.(item1 as alias1 item2)`
    /// — seletivo, itens no escopo direto (com alias opcional).
    Selective { items: Vec<kata_ast::ImportItem> },
}

/// Um módulo importado + como foi importado.
#[derive(Debug, Clone)]
pub struct ImportedModule {
    /// O ResolvedModule do módulo importado (já filtrado por export).
    pub resolved: Arc<ResolvedModule>,
    /// ResolvedModule não-filtrado (com todos os itens) para o driver
    /// rodar inference com acesso a helpers internos.
    pub resolved_unfiltered: Arc<ResolvedModule>,
    /// AST do módulo importado (para `infer_module` no driver).
    pub module_ast: Module,
    /// Como foi importado.
    pub import_kind: ImportKind,
    /// Nome do módulo (último componente do path — ex: "matematica"
    /// para `import utilidades.matematica`). Usado como `origin`
    /// ao copiar tipos para o módulo importador.
    pub module_name: String,
}

/// Cache interno: AST + ResolvedModule não-filtrado.
/// `filter_exports` é aplicado por `load_imports` ao construir `ImportedModule`.
#[derive(Debug)]
struct CachedModule {
    module: Module,
    resolved: Arc<ResolvedModule>,
}

/// Carregador de módulos com cache e cycle detection.
pub struct ModuleLoader {
    /// Cache de módulos já carregados: path → CachedModule (não-filtrado).
    cache: HashMap<PathBuf, Arc<CachedModule>>,
    /// Paths em processo de loading (para detectar ciclos).
    loading: HashSet<PathBuf>,
    /// Diretórios de busca para módulos.
    search_paths: Vec<PathBuf>,
}

impl ModuleLoader {
    /// Cria um novo loader com search paths dados.
    /// O primeiro path deve ser o diretório do arquivo atual.
    pub fn new(search_paths: Vec<PathBuf>) -> Self {
        ModuleLoader {
            cache: HashMap::new(),
            loading: HashSet::new(),
            search_paths,
        }
    }

    /// Carrega um módulo pelo nome (ex: "utilidades.matematica").
    ///
    /// Procura `utilidades/matematica.kata` em cada search path.
    /// Se já está no cache, retorna a versão cacheada.
    /// Se está em loading, detecta ciclo.
    ///
    /// Retorna o `ResolvedModule` **não-filtrado** (com todos os itens).
    /// O `filter_exports` é aplicado por `load_imports` ao construir `ImportedModule`.
    pub fn load(&mut self, module_path: &[String]) -> Result<Arc<ResolvedModule>, LoadError> {
        let resolved_path = self.resolve_path(module_path)?;
        let cached = self.load_path(&resolved_path)?;
        Ok(cached.resolved.clone())
    }

    /// Carrega todos os módulos importados por um módulo.
    ///
    /// Itera sobre `module.items`, encontra cada `Item::ImportDecl`,
    /// carrega o módulo correspondente via `self.load_path()`, filtra por exports,
    /// e retorna a lista de `ImportedModule`.
    pub fn load_imports(&mut self, module: &Module) -> Result<Vec<ImportedModule>, LoadError> {
        let mut imports = Vec::new();
        for item in &module.items {
            if let Item::ImportDecl { path, alias, items } = &item.node {
                let resolved_path = self.resolve_path(path)?;
                let cached = self.load_path(&resolved_path)?;
                let module_name = path.last().cloned().unwrap_or_default();

                // Aplicar filter_exports para obter a versão filtrada (visível
                // para o importador). O não-filtrado fica para inference.
                let filtered = filter_exports((*cached.resolved).clone(), &cached.module);
                let resolved_filtered = Arc::new(filtered);
                let resolved_unfiltered = cached.resolved.clone();

                let import_kind = match (alias, items) {
                    (Some(alias_name), _) => ImportKind::WholeModuleAliased {
                        alias: alias_name.clone(),
                    },
                    (None, Some(item_names)) => ImportKind::Selective {
                        items: item_names.clone(),
                    },
                    (None, None) => {
                        // Módulo inteiro — prefixo é o último componente do path.
                        let prefix = path.last().cloned().unwrap_or_default();
                        ImportKind::WholeModule { prefix }
                    }
                };
                imports.push(ImportedModule {
                    resolved: resolved_filtered,
                    resolved_unfiltered,
                    module_ast: cached.module.clone(),
                    import_kind,
                    module_name,
                });
            }
        }
        Ok(imports)
    }

    /// Carrega um módulo pelo path do arquivo.
    ///
    /// Retorna `Arc<CachedModule>` contendo o AST + ResolvedModule não-filtrado.
    /// O cache armazena a versão não-filtrada para que `load_imports` possa
    /// aplicar `filter_exports` e ainda dar acesso ao não-filtrado para inference.
    fn load_path(&mut self, path: &Path) -> Result<Arc<CachedModule>, LoadError> {
        // Cache hit
        if let Some(cached) = self.cache.get(path) {
            return Ok(cached.clone());
        }

        // Cycle detection
        if self.loading.contains(path) {
            return Err(LoadError::CircularImport {
                path: path.display().to_string(),
            });
        }

        self.loading.insert(path.to_path_buf());

        // Lê o arquivo
        let source = std::fs::read_to_string(path).map_err(|e| {
            self.loading.remove(path);
            LoadError::Io(e.to_string())
        })?;

        // Lex → Parse → Resolve
        let tokens = lex(&source).map_err(|e| {
            self.loading.remove(path);
            LoadError::Lex(e)
        })?;
        let module = parse(tokens).map_err(|e| {
            self.loading.remove(path);
            LoadError::Parse(e)
        })?;
        let module_name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let resolved = resolve_with_origin(&module, &module_name).map_err(|e| {
            self.loading.remove(path);
            LoadError::Resolve(e)
        })?;

        // Sub-módulos precisam do prelude: Int, Float, +, etc.
        // Sem isso, o TypeEnv do sub-módulo só tem Unit, e qualquer
        // tipo do prelude falha com UnboundName.
        // Exceção: core.kata É o prelude — não injeta em si mesmo.
        let is_core = path.file_stem().is_some_and(|s| s == "core");
        let merged = if is_core {
            resolved
        } else {
            let prelude = crate::prelude_sigs::load_prelude().map_err(|e| {
                self.loading.remove(path);
                LoadError::Resolve(e)
            })?;
            merge_two(prelude, resolved)
        };

        self.loading.remove(path);

        // Cache armazena não-filtrado — filter_exports é aplicado por load_imports.
        let cached = Arc::new(CachedModule {
            module,
            resolved: Arc::new(merged),
        });
        self.cache.insert(path.to_path_buf(), cached.clone());
        Ok(cached)
    }

    /// Resolve um caminho de módulo (ex: `["utilidades", "matematica"]`)
    /// para um path de filesystem: `utilidades/matemática.kata`.
    fn resolve_path(&self, module_path: &[String]) -> Result<PathBuf, LoadError> {
        let mut relative = PathBuf::new();
        for (i, part) in module_path.iter().enumerate() {
            if i + 1 == module_path.len() {
                // Último componente: adiciona extensão .kata
                relative.push(format!("{part}.kata"));
            } else {
                relative.push(part);
            }
        }

        for search_path in &self.search_paths {
            let candidate = search_path.join(&relative);
            if candidate.exists() {
                return Ok(candidate);
            }
        }

        Err(LoadError::NotFound {
            path: module_path.join("."),
        })
    }

    /// Limpa o cache (útil para testes).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

/// Filtra um `ResolvedModule` para só conter itens exportados.
///
/// Princípio do manual (§3.1): "Apenas o que é exportado pode ser
/// importado por outros módulos."
///
/// Módulo sem `ExportDecl` = aberto (tudo exportado).
/// Módulo com `ExportDecl` = só itens exportados são visíveis.
///
/// Export de tipo é transitivo: leva ImplEntry + interfaces + métodos
/// + supertraits dessas interfaces (ver PRD §3.4.1).
pub fn filter_exports(resolved: ResolvedModule, module: &Module) -> ResolvedModule {
    // Coletar nomes exportados: percorrer module.items por ExportDecl.
    let exported: HashSet<String> = module
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::ExportDecl { items } => Some(
                items
                    .iter()
                    .filter(|ei| ei.reexport_from.is_none())
                    .map(|ei| ei.name.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();

    // Se não há export decl, TUDO é exportado (compatibilidade — módulo
    // sem export é módulo aberto, como o prelude atual).
    if exported.is_empty() {
        return resolved; // sem filtro
    }

    // ── Fechamento transitivo do export ──────────────────────────
    // Para cada tipo exportado, coletar dependências transitivas:
    // - ImplEntry onde type_name == tipo exportado
    // - Interface names dessas impls (interfaces implementadas)
    // - Métodos dessas impls (signatures + functions)
    // - Interfaces definidas no módulo que essas impls referenciam
    //   (incluindo supertraits recursivamente)

    let mut closure = exported.clone();

    // 1. Para cada tipo exportado, encontrar impls e adicionar nomes
    //    de interfaces + nomes de métodos ao closure.
    for impl_entry in resolved.interface_registry.impls_view() {
        if closure.contains(&impl_entry.type_name) {
            // Adicionar interface implementada
            closure.insert(impl_entry.interface_name.clone());
            // Adicionar nomes de métodos
            for method in &impl_entry.methods {
                closure.insert(method.name.clone());
            }
        }
    }

    // 2. Interfaces definidas no módulo que estão no closure:
    //    adicionar suas supertraits (recursivo até fixpoint).
    let mut changed = true;
    while changed {
        changed = false;
        for iface_name in closure.iter().cloned().collect::<Vec<_>>() {
            if let Some(info) = resolved.interface_registry.get_interface(&iface_name) {
                for st in &info.supertraits {
                    if closure.insert(st.clone()) {
                        changed = true;
                    }
                }
            }
        }
    }

    // ── Filtrar ResolvedModule pelo closure ──────────────────────

    // Signatures: manter se nome está no closure (função exportada
    // diretamente OU método de impl de tipo exportado).
    let signatures: Vec<_> = resolved
        .signatures
        .into_iter()
        .filter(|s| closure.contains(&s.name))
        .collect();

    // Functions: mesmo critério (corpos Kata de métodos de interface).
    let functions: Vec<_> = resolved
        .functions
        .into_iter()
        .filter(|f| closure.contains(&f.name))
        .collect();

    // Actions: só se explicitamente exportadas.
    let actions: Vec<_> = resolved
        .actions
        .into_iter()
        .filter(|a| closure.contains(&a.name))
        .collect();

    // Filtrar registries pelo closure: tipos não exportados pelo módulo
    // ficam invisíveis para importadores. Tipos do prelude (origin "core")
    // sempre passam.
    let mut type_env = resolved.type_env;
    type_env.retain_by_closure(&closure);

    let mut enum_registry = resolved.enum_registry;
    enum_registry.retain_by_closure(&closure);

    let mut struct_registry = resolved.struct_registry;
    struct_registry.retain_by_closure(&closure);

    let mut interface_registry = resolved.interface_registry;
    interface_registry.retain_by_closure(&closure);

    ResolvedModule {
        signatures,
        functions,
        actions,
        type_env,
        enum_registry,
        struct_registry,
        interface_registry,
        ..resolved
    }
}

#[cfg(test)]
mod tests {
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
        let resolved = loader.load(&["simple".into()]).unwrap();

        // 42 é a entry expr — não há assinaturas do usuário, mas o prelude
        // é injetado, então signatures contém as do prelude (ex: +, -, *).
        assert!(!resolved.signatures.is_empty());
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
        let resolved = loader.load(&["util".into(), "math".into()]).unwrap();
        // A assinatura do usuário (+) está entre as signatures (junto com prelude).
        assert!(resolved.signatures.iter().any(|s| s.name == "+"));
    }

    #[test]
    fn cache_returns_same_module() {
        let tmp = tempfile::tempdir().unwrap();
        create_temp_file(tmp.path(), "cached.kata", "42");

        let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
        let first = loader.load(&["cached".into()]).unwrap();
        let second = loader.load(&["cached".into()]).unwrap();
        // Arc clones — mesmo ponteiro
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn not_found_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let mut loader = ModuleLoader::new(vec![tmp.path().to_path_buf()]);
        let err = loader.load(&["nonexistent".into()]).unwrap_err();
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

        let mut loader =
            ModuleLoader::new(vec![tmp1.path().to_path_buf(), tmp2.path().to_path_buf()]);
        let resolved = loader.load(&["found".into()]).unwrap();
        // Prelude injetado — signatures não está vazio.
        assert!(!resolved.signatures.is_empty());
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

        let imports = loader.load_imports(&module).unwrap();
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
        let resolved = loader.load(&["open".into()]).unwrap();
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
        let resolved = loader.load(&["filtered".into()]).unwrap();
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
}
