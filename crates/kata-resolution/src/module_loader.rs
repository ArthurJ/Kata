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

use crate::{merge_two, resolve, ResolvedModule};

/// Erro de carregamento de módulo.
#[derive(Debug, Clone)]
pub enum LoadError {
    /// Arquivo não encontrado em nenhum search path.
    NotFound { path: String },
    /// Erro de lex.
    LexError(String),
    /// Erro de parse.
    ParseError(String),
    /// Erro de resolution.
    ResolveError(String),
    /// Ciclo de import detectado.
    CircularImport { path: String },
    /// Erro de I/O.
    IoError(String),
}

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
    /// Como foi importado.
    pub import_kind: ImportKind,
}

/// Carregador de módulos com cache e cycle detection.
pub struct ModuleLoader {
    /// Cache de módulos já carregados: path → ResolvedModule.
    cache: HashMap<PathBuf, Arc<ResolvedModule>>,
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
    pub fn load(&mut self, module_path: &[String]) -> Result<Arc<ResolvedModule>, LoadError> {
        let resolved_path = self.resolve_path(module_path)?;
        self.load_path(&resolved_path)
    }

    /// Carrega todos os módulos importados por um módulo.
    ///
    /// Itera sobre `module.items`, encontra cada `Item::ImportDecl`,
    /// carrega o módulo correspondente via `self.load()`, filtra por exports,
    /// e retorna a lista de `ImportedModule`.
    pub fn load_imports(&mut self, module: &Module) -> Result<Vec<ImportedModule>, LoadError> {
        let mut imports = Vec::new();
        for item in &module.items {
            if let Item::ImportDecl {
                path,
                alias,
                items,
            } = &item.node
            {
                let resolved = self.load(path)?;
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
                    resolved,
                    import_kind,
                });
            }
        }
        Ok(imports)
    }

    /// Carrega um módulo pelo path do arquivo.
    pub fn load_path(&mut self, path: &Path) -> Result<Arc<ResolvedModule>, LoadError> {
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
            LoadError::IoError(e.to_string())
        })?;

        // Lex → Parse → Resolve
        let tokens = lex(&source).map_err(|e| {
            self.loading.remove(path);
            LoadError::LexError(format!("{e:?}"))
        })?;
        let module = parse(tokens).map_err(|e| {
            self.loading.remove(path);
            LoadError::ParseError(format!("{e:?}"))
        })?;
        let resolved = resolve(&module).map_err(|e| {
            self.loading.remove(path);
            LoadError::ResolveError(format!("{e:?}"))
        })?;

        // Sub-módulos precisam do prelude: Int, Float, +, etc.
        // Sem isso, o TypeEnv do sub-módulo só tem Unit, e qualquer
        // tipo do prelude falha com UnboundName.
        let prelude = crate::prelude_sigs::load_prelude().map_err(|e| {
            self.loading.remove(path);
            LoadError::ResolveError(format!("erro ao carregar prelude para sub-módulo: {e:?}"))
        })?;
        let merged = merge_two(prelude, resolved);

        // Filtrar por exports: só itens exportados são visíveis para importadores.
        let filtered = filter_exports(merged, &module);

        self.loading.remove(path);

        let filtered = Arc::new(filtered);
        self.cache.insert(path.to_path_buf(), filtered.clone());
        Ok(filtered)
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

    ResolvedModule {
        signatures,
        functions,
        actions,
        // type_env, enum_registry, struct_registry, interface_registry,
        // refined_decls, enum_pred_decls, refines_registry — mantidos
        // sem filtro por ora. Tipos são sempre exportados quando o tipo
        // está no closure; o filtro fino de registries é evolução futura.
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
        let sig_names: Vec<&str> = resolved.signatures.iter().map(|s| s.name.as_str()).collect();
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
        let resolved = loader.load(&["filtered".into()]).unwrap();
        // Só public_fn visível (private_fn filtrada pelo export).
        // Prelude também está presente mas não tem public_fn/private_fn.
        assert!(resolved.signatures.iter().any(|s| s.name == "public_fn"));
        assert!(!resolved.signatures.iter().any(|s| s.name == "private_fn"));
    }
}