//! Module Loader — carrega módulos do filesystem com cache e cycle detection.
//!
//! Infraestrutura para `import modulo.submodulo`.
//! O loader resolve paths, parsea, resolve, filtra exports, e cacheia o resultado.
//! Ciclos de import são detectados via HashSet de paths em loading.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kata_ast::{
    DotIndex, Expr, GuardClause, Item, LambdaClause, Module, ReadMode, SelectArm, Spanned,
};
use kata_lexer::lex;
use kata_parser::parse;

use crate::{FunctionDef, ResolvedModule, merge_two, resolve_with_origin};

// ── Stdlib embedded no binário ───────────────────────────

/// Código fonte dos arquivos da stdlib, embutidos via `include_str!`.
/// O ModuleLoader lê destes em vez do filesystem.
mod embedded {
    pub const MOD: &str = include_str!("../../../stdlib/mod.kata");
    pub const CORE: &str = include_str!("../../../stdlib/core.kata");
    pub const CORE_INTERNALS: &str = include_str!("../../../stdlib/core_internals.kata");
    pub const MATH: &str = include_str!("../../../stdlib/math.kata");
    pub const COMPLEX: &str = include_str!("../../../stdlib/complex.kata");
    pub const STDIO: &str = include_str!("../../../stdlib/stdio.kata");
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

/// Verifica se um path é sintético (embedded stdlib).
fn is_stdlib_path(path: &Path) -> bool {
    path.starts_with(STDLIB_PREFIX)
}

/// Constrói um path sintético para um módulo stdlib embedded.
fn stdlib_synthetic_path(name: &str) -> PathBuf {
    PathBuf::from(format!("{STDLIB_PREFIX}/{name}.kata"))
}

/// Tenta resolver um nome contra os módulos stdlib embedded.
/// Retorna o path sintético se o módulo existe.
fn try_resolve_embedded(normal_path: &[String]) -> Option<PathBuf> {
    let last = normal_path.last()?;
    if embedded_source(last).is_some() && normal_path.len() == 1 {
        return Some(stdlib_synthetic_path(last));
    }
    None
}

// ── Prefixos de import path ──────────────────────────────

/// Prefixo especial detectado no início de um import path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PrefixKind {
    /// `super` repetido n vezes — `super.X` (n=1), `super.super.X` (n=2)
    Super(usize),
    /// `stdlib` — resolve na stdlib built-in
    Stdlib,
    /// Sem prefixo — comportamento normal (entry_dir + stdlib fallback)
    None,
}

/// Separa o prefixo especial do restante do path.
///
/// `["super", "calculus"]` → `(Super(1), ["calculus"])`
/// `["super", "super", "utils"]` → `(Super(2), ["utils"])`
/// `["stdlib", "math"]` → `(Stdlib, ["math"])`
/// `["math", "algebra"]` → `(None, ["math", "algebra"])`
fn split_import_prefix(path: &[String]) -> (PrefixKind, &[String]) {
    if path.is_empty() {
        return (PrefixKind::None, path);
    }

    if path[0] == "super" {
        let n = path.iter().take_while(|s| s == &"super").count();
        return (PrefixKind::Super(n), &path[n..]);
    }

    if path[0] == "stdlib" {
        return (PrefixKind::Stdlib, &path[1..]);
    }

    (PrefixKind::None, path)
}

/// Constrói um path relativo a partir de componentes normais.
///
/// `["math", "algebra"]` → `math/algebra.kata`
/// `["calculus"]` → `calculus.kata`
/// `["math", "vectors", "vec2"]` → `math/vectors/vec2.kata`
///
/// O último componente vira `component.kata`. Componentes intermediários
/// são diretórios (namespaces).
fn build_relative_path(normal_path: &[String]) -> PathBuf {
    let mut relative = PathBuf::new();
    for (i, part) in normal_path.iter().enumerate() {
        if i + 1 == normal_path.len() {
            relative.push(format!("{part}.kata"));
        } else {
            relative.push(part);
        }
    }
    relative
}

/// Tenta resolver `normal_path` contra `base`.
///
/// Para o último componente `C`:
/// - Se `base/.../C.kata` existe → arquivo módulo
/// - Se `base/.../C/` é diretório com `mod.kata` → diretório módulo
/// - Caso contrário → `None` (não encontrado)
fn try_resolve(base: &Path, normal_path: &[String]) -> Option<PathBuf> {
    // 1. Tenta como arquivo: base/.../last.kata
    let file_path = base.join(build_relative_path(normal_path));
    if file_path.exists() {
        return Some(file_path);
    }
    // 2. Se o último componente é diretório, tenta mod.kata
    if let Some(last) = normal_path.last() {
        let mut dir = base.to_path_buf();
        for part in &normal_path[..normal_path.len() - 1] {
            dir.push(part);
        }
        dir.push(last);
        if dir.is_dir() {
            let mod_path = dir.join("mod.kata");
            if mod_path.exists() {
                return Some(mod_path);
            }
        }
    }
    None
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
/// Walker recursivo: coleta todos os `Ident { name }` em uma expressão.
/// Usado para descobrir dependências internas de corpos de funções exportadas.
fn collect_idents(expr: &Spanned<Expr>, out: &mut HashSet<String>) {
    match &expr.node {
        Expr::Ident { name } => {
            out.insert(name.clone());
        }
        Expr::Apply { callee, args } => {
            collect_idents(callee, out);
            for arg in args {
                collect_idents(arg, out);
            }
        }
        Expr::TypeAscription { expr, .. } => collect_idents(expr, out),
        Expr::Grouping { inner } => collect_idents(inner, out),
        Expr::Tuple { elements } => {
            for el in elements {
                collect_idents(el, out);
            }
        }
        Expr::Let { value, .. } => collect_idents(value, out),
        Expr::LetDestruct { value, .. } => collect_idents(value, out),
        Expr::VariantQual { .. } => {}
        Expr::Lambda {
            body,
            guards,
            with_bindings,
            ..
        } => {
            collect_idents(body, out);
            for g in guards {
                collect_guard_idents(g, out);
            }
            for wb in with_bindings {
                collect_idents(&wb.value, out);
            }
        }
        Expr::Match { scrutinee, arms } => {
            collect_idents(scrutinee, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    collect_idents(g, out);
                }
                collect_idents(&arm.body, out);
            }
        }
        Expr::Hole => {}
        Expr::Pipe { lhs, rhs } => {
            collect_idents(lhs, out);
            collect_idents(rhs, out);
        }
        Expr::PipeLimit { lhs, rhs, limit } => {
            collect_idents(lhs, out);
            collect_idents(rhs, out);
            collect_idents(limit, out);
        }
        Expr::ActionCall { args, .. } => collect_idents(args, out),
        Expr::TypeOf { expr } => collect_idents(expr, out),
        Expr::Return(expr) => collect_idents(expr, out),
        Expr::Loop { body } => {
            for stmt in body {
                collect_idents(stmt, out);
            }
        }
        Expr::Break | Expr::Continue => {}
        Expr::Var { value, .. } => collect_idents(value, out),
        Expr::Reassign { value, .. } => collect_idents(value, out),
        Expr::Question(expr) => collect_idents(expr, out),
        Expr::PipeFallback { lhs, rhs } => {
            collect_idents(lhs, out);
            collect_idents(rhs, out);
        }
        Expr::DotAccess { expr, index } => {
            collect_idents(expr, out);
            if let DotIndex::Range { start, end, .. } = index {
                collect_idents(start, out);
                collect_idents(end, out);
            }
        }
        Expr::ListLit { elements } => {
            for el in elements {
                collect_idents(el, out);
            }
        }
        Expr::ArrayLit { elements } => {
            for el in elements {
                collect_idents(el, out);
            }
        }
        Expr::DictLit { entries } => {
            for (k, v) in entries {
                collect_idents(k, out);
                collect_idents(v, out);
            }
        }
        Expr::SetLit { elements } => {
            for el in elements {
                collect_idents(el, out);
            }
        }
        Expr::RangeLit {
            start, step, end, ..
        } => {
            collect_idents(start, out);
            collect_idents(step, out);
            collect_idents(end, out);
        }
        Expr::ForIn { iterable, body, .. } => {
            collect_idents(iterable, out);
            for stmt in body {
                collect_idents(stmt, out);
            }
        }
        Expr::In { item, collection } => {
            collect_idents(item, out);
            collect_idents(collection, out);
        }
        Expr::ChannelSend { channel, value } => {
            collect_idents(channel, out);
            collect_idents(value, out);
        }
        Expr::ChannelRecv { channel, .. } => collect_idents(channel, out),
        Expr::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            for arm in arms {
                collect_select_arm_idents(arm, out);
            }
            if let Some(t) = timeout_ms {
                collect_idents(t, out);
            }
            if let Some(t) = timeout_body {
                collect_idents(t, out);
            }
        }
        Expr::Block { stmts } => {
            for stmt in stmts {
                collect_idents(stmt, out);
            }
        }
        // Literais não contêm idents.
        Expr::IntLit { .. }
        | Expr::FloatLit { .. }
        | Expr::TextLit { .. }
        | Expr::BytesLit { .. }
        | Expr::Unit => {}
    }
}

fn collect_guard_idents(guard: &GuardClause, out: &mut HashSet<String>) {
    if let Some(cond) = &guard.condition {
        collect_idents(cond, out);
    }
    collect_idents(&guard.body, out);
}

fn collect_select_arm_idents(arm: &SelectArm, out: &mut HashSet<String>) {
    match arm {
        SelectArm::Channel { channel, body, .. } => {
            collect_idents(channel, out);
            collect_idents(body, out);
        }
        SelectArm::IoRead {
            handle_expr,
            read_mode,
            body,
            ..
        } => {
            collect_idents(handle_expr, out);
            if let ReadMode::Chunk(n) = read_mode {
                collect_idents(n, out);
            }
            collect_idents(body, out);
        }
    }
}

/// Walker sobre `LambdaClause` — coleta idents do body, guards e with_bindings.
fn collect_clause_idents(clause: &Spanned<LambdaClause>, out: &mut HashSet<String>) {
    collect_idents(&clause.node.body, out);
    for g in &clause.node.guards {
        collect_guard_idents(g, out);
    }
    for wb in &clause.node.with_bindings {
        collect_idents(&wb.value, out);
    }
}

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
                collect_clause_idents(clause, &mut idents);
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

        let mut loader =
            ModuleLoader::new(vec![tmp1.path().to_path_buf(), tmp2.path().to_path_buf()]);
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
}
