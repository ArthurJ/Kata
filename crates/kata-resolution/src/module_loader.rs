//! Module Loader — carrega módulos do filesystem com cache e cycle detection.
//!
//! Fio 7 Fase 3: infraestrutura para `import modulo.submodulo`.
//! O loader resolve paths, parsea, resolve, e cacheia o resultado.
//! Ciclos de import são detectados via HashSet de paths em loading.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kata_lexer::lex;
use kata_parser::parse;

use crate::{ResolvedModule, resolve};

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

        self.loading.remove(path);

        let resolved = Arc::new(resolved);
        self.cache.insert(path.to_path_buf(), resolved.clone());
        Ok(resolved)
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

        // 42 é a entry expr — não há assinaturas, mas o módulo resolve.
        assert!(resolved.signatures.is_empty());
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
        assert_eq!(resolved.signatures.len(), 1);
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
        assert!(resolved.signatures.is_empty());
    }
}
