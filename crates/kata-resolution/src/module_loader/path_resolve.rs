//! Path resolution for module imports.
//!
//! Funções puras que resolvem nomes de módulos em paths de filesystem.
//! Não dependem de estado do `ModuleLoader` — recebem strings/paths e
//! retornam `PathBuf`/`Option`.

use std::path::{Path, PathBuf};

use super::{STDLIB_PREFIX, embedded_source};

// ── Stdlib path helpers ──────────────────────────────────

/// Verifica se um path é sintético (embedded stdlib).
pub(crate) fn is_stdlib_path(path: &Path) -> bool {
    path.starts_with(STDLIB_PREFIX)
}

/// Constrói um path sintético para um módulo stdlib embedded.
pub(crate) fn stdlib_synthetic_path(name: &str) -> PathBuf {
    PathBuf::from(format!("{STDLIB_PREFIX}/{name}.kata"))
}

/// Tenta resolver um nome contra os módulos stdlib embedded.
/// Retorna o path sintético se o módulo existe.
pub(crate) fn try_resolve_embedded(normal_path: &[String]) -> Option<PathBuf> {
    let last = normal_path.last()?;
    if embedded_source(last).is_some() && normal_path.len() == 1 {
        return Some(stdlib_synthetic_path(last));
    }
    None
}

// ── Prefixos de import path ──────────────────────────────

/// Prefixo especial detectado no início de um import path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PrefixKind {
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
pub(crate) fn split_import_prefix(path: &[String]) -> (PrefixKind, &[String]) {
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
pub(crate) fn build_relative_path(normal_path: &[String]) -> PathBuf {
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
pub(crate) fn try_resolve(base: &Path, normal_path: &[String]) -> Option<PathBuf> {
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
