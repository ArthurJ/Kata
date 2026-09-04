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

use crate::{FunctionDef, ResolvedModule, merge_two, resolve_with_origin};

mod path_resolve;
use path_resolve::{
    PrefixKind, is_stdlib_path, split_import_prefix, stdlib_synthetic_path, try_resolve,
    try_resolve_embedded,
};

// ── Stdlib embedded no binário ───────────────────────────

/// Código fonte dos arquivos da stdlib, embutidos via `include_str!`.
/// O ModuleLoader lê destes em vez do filesystem.
mod embedded {
    pub const MOD: &str = include_str!("../../../../stdlib/mod.kata");
    pub const CORE: &str = include_str!("../../../../stdlib/core.kata");
    pub const CORE_INTERNALS: &str = include_str!("../../../../stdlib/core_internals.kata");
    pub const MATH: &str = include_str!("../../../../stdlib/math.kata");
    pub const COMPLEX: &str = include_str!("../../../../stdlib/complex.kata");
    pub const STDIO: &str = include_str!("../../../../stdlib/stdio.kata");
}

/// Prefixo sintético para paths de stdlib embedded.
const STDLIB_PREFIX: &str = "$stdlib";

/// Mapa de nome do módulo (sem extensão) → código fonte embedded.
fn embedded_source(name: &str) -> Option<&'static str> {
    match name {
        "mod" => Some(embedded::MOD),
        "core" => Some(embedded::CORE),
        "core_internals" => Some(embedded::CORE_INTERNALS),
        "math" => Some(embedded::MATH),
        "complex" => Some(embedded::COMPLEX),
        "stdio" => Some(embedded::STDIO),
        _ => None,
    }
}

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
    /// Nome de módulo reservado (`stdlib`).
    ReservedName { name: String },
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
            LoadError::ReservedName { name } => {
                write!(
                    f,
                    "não é possível nomear um módulo como `{name}` — nome reservado"
                )
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
    /// Stdlib pré-carregada (core + core_internals) para injetar tipos
    /// primitivos em user modules durante `load_path`. `None` no loader
    /// temporário interno de `load_stdlib_embedded`.
    stdlib: Option<Arc<ResolvedModule>>,
}

impl ModuleLoader {
    /// Cria um novo loader com search paths dados.
    /// O primeiro path deve ser o diretório do arquivo atual.
    /// Pré-carrega a stdlib embedded (core + core_internals) para que
    /// `load_path` de user modules tenha tipos primitivos no TypeEnv.
    pub fn new(search_paths: Vec<PathBuf>) -> Self {
        let stdlib = Self::load_stdlib_embedded();
        ModuleLoader {
            cache: HashMap::new(),
            loading: HashSet::new(),
            search_paths,
            stdlib,
        }
    }

    /// Carrega a stdlib embedded (core + core_internals) sem filesystem.
    /// Cria um `ModuleLoader` temporário com `stdlib: None` para evitar
    /// recursão. O `load_path` de `core.kata` (stdlib path) não tenta
    /// `merge_two` porque `is_stdlib_path` é true.
    fn load_stdlib_embedded() -> Option<Arc<ResolvedModule>> {
        let mut loader = ModuleLoader {
            cache: HashMap::new(),
            loading: HashSet::new(),
            search_paths: Vec::new(),
            stdlib: None,
        };
        loader
            .load(&["stdlib".into(), "core".into()], Path::new(STDLIB_PREFIX))
            .ok()
    }

    /// Carrega um módulo pelo nome (ex: "utilidades.matematica").
    ///
    /// Procura `utilidades/matematica.kata` em cada search path.
    /// Se já está no cache, retorna a versão cacheada.
    /// Se está em loading, detecta ciclo.
    ///
    /// Retorna o `ResolvedModule` **não-filtrado** (com todos os itens).
    /// O `filter_exports` é aplicado por `load_imports` ao construir `ImportedModule`.
    pub fn load(
        &mut self,
        module_path: &[String],
        entry_dir: &Path,
    ) -> Result<Arc<ResolvedModule>, LoadError> {
        let resolved_path = self.resolve_path(module_path, entry_dir)?;
        let cached = self.load_path(&resolved_path)?;
        Ok(cached.resolved.clone())
    }

    /// Carrega todos os módulos importados por um módulo.
    ///
    /// Itera sobre `module.items`, encontra cada `Item::ImportDecl`,
    /// carrega o módulo correspondente via `self.load_path()`, filtra por exports,
    /// e retorna a lista de `ImportedModule`.
    ///
    /// `entry_dir` é o diretório do arquivo importador — usado para resolver
    /// `super.` nos imports deste módulo.
    pub fn load_imports(
        &mut self,
        module: &Module,
        entry_dir: &Path,
    ) -> Result<Vec<ImportedModule>, LoadError> {
        let mut imports = Vec::new();
        for item in &module.items {
            if let Item::ImportDecl { path, alias, items } = &item.node {
                let resolved_path = self.resolve_path(path, entry_dir)?;
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
                        // Para paths com prefixo super/stdlib, o prefixo visível
                        // é o último componente normal (não "super"/"stdlib").
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

        // Proibir `stdlib` como nome de módulo — é um namespace reservado.
        if path.file_stem().is_some_and(|s| s == "stdlib") {
            self.loading.remove(path);
            return Err(LoadError::ReservedName {
                name: "stdlib".to_string(),
            });
        }

        // Lê o arquivo — embedded stdlib ou filesystem.
        let source = if is_stdlib_path(path) {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            embedded_source(&name)
                .ok_or_else(|| {
                    self.loading.remove(path);
                    LoadError::NotFound {
                        path: path.display().to_string(),
                    }
                })?
                .to_string()
        } else {
            std::fs::read_to_string(path).map_err(|e| {
                self.loading.remove(path);
                LoadError::Io(e.to_string())
            })?
        };

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

        // Stdlib embedded: processa imports recursivamente (ex: core.kata
        // importa core_internals).
        // User modules: injeta stdlib pré-carregada (merge_two) para que
        // tipos primitivos (Int, Float, etc.) estejam no TypeEnv. Sem isso,
        // `resolve_with_origin` resolve `Int` como `Ty::Var("Int")` em vez
        // de `Ty::Prim(Int)`, e o inference não despacha operadores.
        let mut merged = if is_stdlib_path(path) {
            resolved
        } else if let Some(stdlib) = self.stdlib.clone() {
            merge_two((*stdlib).clone(), resolved)
        } else {
            // stdlib não carregada (loader temporário) — sem injeção.
            resolved
        };

        // Carregar imports do módulo recursivamente e fazer merge.
        // Stdlib embedded usa diretório sintético; user modules usam
        // o diretório do arquivo importador.
        let entry_dir = if is_stdlib_path(path) {
            PathBuf::from(STDLIB_PREFIX)
        } else {
            path.parent().unwrap_or(Path::new(".")).to_path_buf()
        };
        let imports = self.load_imports(&module, &entry_dir)?;
        if !imports.is_empty() {
            crate::merge_imports::merge_imports(&mut merged, &imports);
        }

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
    ///
    /// Aceita prefixos especiais no path:
    /// - `["super", ...]` — sobe um nível de `entry_dir` por `super`
    /// - `["super", "super", ...]` — sobe dois níveis
    /// - `["stdlib", ...]` — resolve na stdlib built-in
    /// - `["modulo", ...]` — procura em `entry_dir` primeiro, stdlib como fallback
    ///
    /// Detecção de `mod.kata`: se o último componente é um diretório `D`,
    /// carrega `D/mod.kata`. Se `D` não tem `mod.kata`, é `NotFound`.
    /// Componentes intermediários que são diretórios são namespaces —
    /// a navegação continua sem precisar de `mod.kata`.
    fn resolve_path(&self, module_path: &[String], entry_dir: &Path) -> Result<PathBuf, LoadError> {
        if module_path.is_empty() {
            return Err(LoadError::NotFound {
                path: String::new(),
            });
        }

        // Detectar e separar prefixo especial
        let (prefix_kind, normal_path) = split_import_prefix(module_path);

        // `import stdlib` (sem subpath) resolve para stdlib/mod.kata.
        // Outros prefixos com path vazio são erro.
        if normal_path.is_empty() && prefix_kind != PrefixKind::Stdlib {
            return Err(LoadError::NotFound {
                path: module_path.join("."),
            });
        }

        match prefix_kind {
            PrefixKind::Super(n) => {
                // Subir n níveis de entry_dir
                let mut base = entry_dir.to_path_buf();
                for _ in 0..n {
                    if !base.pop() {
                        return Err(LoadError::NotFound {
                            path: module_path.join("."),
                        });
                    }
                }
                // Procurar SÓ no base resolvido (não fallback stdlib)
                try_resolve(&base, normal_path).ok_or_else(|| LoadError::NotFound {
                    path: module_path.join("."),
                })
            }
            PrefixKind::Stdlib => {
                // Stdlib embedded no binário — path sintético.
                // `import stdlib` (path vazio) resolve para mod.kata.
                if normal_path.is_empty() {
                    Ok(stdlib_synthetic_path("mod"))
                } else {
                    try_resolve_embedded(normal_path).ok_or_else(|| LoadError::NotFound {
                        path: module_path.join("."),
                    })
                }
            }
            PrefixKind::None => {
                // entry_dir primeiro (diretório do arquivo importador),
                // depois search_paths, depois stdlib embedded como fallback.
                if let Some(found) = try_resolve(entry_dir, normal_path) {
                    return Ok(found);
                }
                for search_path in &self.search_paths {
                    if let Some(found) = try_resolve(search_path, normal_path) {
                        return Ok(found);
                    }
                }
                // Fallback: stdlib embedded.
                if let Some(found) = try_resolve_embedded(normal_path) {
                    return Ok(found);
                }
                Err(LoadError::NotFound {
                    path: module_path.join("."),
                })
            }
        }
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

    // ── Fechamento interno: dependências de corpos de funções ────
    // Funções exportadas podem chamar funções não-exportadas (ex: `div`
    // chama `bi_div`). Essas dependências internas precisam estar no
    // DispatchTable durante inferência das funções do prelude, mas não
    // são visíveis para o usuário. Coletamos via walk recursivo da AST.
    let all_sig_names: HashSet<String> =
        resolved.signatures.iter().map(|s| s.name.clone()).collect();
    let func_by_name: HashMap<String, &FunctionDef> = resolved
        .functions
        .iter()
        .map(|f| (f.name.clone(), f))
        .collect();

    let mut internal_closure: HashSet<String> = HashSet::new();
    let mut worklist: Vec<String> = closure
        .iter()
        .filter(|n| func_by_name.contains_key(*n))
        .cloned()
        .collect();

    while let Some(fname) = worklist.pop() {
        if let Some(fdef) = func_by_name.get(&fname) {
            let mut idents = HashSet::new();
            for clause in &fdef.clauses {
                crate::ident_collector::collect_clause_idents(clause, &mut idents);
            }
            for ident in idents {
                // Só importa se for uma signature conhecida e não exportada.
                if all_sig_names.contains(&ident)
                    && !closure.contains(&ident)
                    && internal_closure.insert(ident.clone())
                {
                    // Se essa função internal tem corpo, adicionar ao worklist
                    // para escanear suas dependências transitivas.
                    if func_by_name.contains_key(&ident) {
                        worklist.push(ident);
                    }
                }
            }
        }
    }

    // ── Filtrar ResolvedModule pelo closure ──────────────────────

    // Signatures: manter se nome está no closure (função exportada
    // diretamente OU método de impl de tipo exportado).
    // internal_signatures: dependências de corpos, não exportadas.
    let sigs_remaining = resolved.signatures;
    let signatures: Vec<_> = sigs_remaining
        .iter()
        .filter(|s| closure.contains(&s.name))
        .cloned()
        .collect();
    let internal_signatures: Vec<_> = sigs_remaining
        .into_iter()
        .filter(|s| internal_closure.contains(&s.name))
        .collect();

    // Functions: mesmo critério (corpos Kata de métodos de interface).
    // Functions internas (não exportadas) também são mantidas para que
    // o inference possa processar seus corpos na Fase 1.
    let functions: Vec<_> = resolved
        .functions
        .into_iter()
        .filter(|f| closure.contains(&f.name) || internal_closure.contains(&f.name))
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
        internal_signatures,
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
mod tests;
