//! Front-end pipeline reusável: lex → parse → resolve → infer (sem codegen).
//!
//! O LSP consome apenas o front-end — sem codegen, sem runtime, sem side-effects.
//! O `FrontendResult` carrega tudo que tooling precisa: AST, ResolvedModule, TAST.
//!
//! Imports multi-arquivo são resolvidos via `ModuleLoader` de `kata-resolution`
//! (API pública). O merge de imports usa `kata_resolution::merge_imports`.

use std::path::Path;

use kata_ast::Module;
use kata_inference::TypedModule;
use kata_resolution::{ImportedModule, ModuleLoader, ResolvedModule, merge_imports};

/// Resultado do front-end — tudo que o LSP precisa.
pub struct FrontendResult {
    #[allow(dead_code, reason = "usado por go-to-def futuro")]
    pub module: Module,
    #[allow(dead_code, reason = "usado por go-to-def futuro")]
    pub resolved: ResolvedModule,
    pub typed: TypedModule,
}

/// Batch de erros do front-end coletados em um único enum.
///
/// Chamado `FrontendBatch` (não `FrontendError`) para evitar colisão
/// conceitual com `kata_diagnostics::FrontendError` (que cobre apenas
/// lex/parse). Este wrapper agrega erros de 4 fontes:
/// - `Lex`/`Parse`: `kata_diagnostics::FrontendError` (com Span)
/// - `Resolve`: `Vec<ResolveError>` (sem Span, mas com código miette)
/// - `Infer`: `kata_diagnostics::MiddleError` (com Span)
///
/// Vive no LSP (não em `kata-diagnostics`) porque carrega `ResolveError`
/// de `kata-resolution`, e `kata-diagnostics` não pode depender de
/// `kata-resolution` (seria ciclo: `kata-diagnostics → kata-resolution
/// → kata-diagnostics`).
pub enum FrontendBatch {
    Lex(kata_diagnostics::FrontendError),
    Parse(kata_diagnostics::FrontendError),
    Resolve(Vec<kata_resolution::ResolveError>),
    Infer(kata_diagnostics::MiddleError),
}

/// Roda lex → parse → resolve(+imports) → infer (sem codegen).
pub fn run_frontend(
    source: &str,
    file_path: Option<&str>,
) -> Result<FrontendResult, Vec<FrontendBatch>> {
    // 1. Lex
    let tokens = kata_lexer::lex(source).map_err(|e| vec![FrontendBatch::Lex(e)])?;

    // 2. Parse (com recovery — acumula erros de top-level items)
    let (module, parse_errors) = kata_parser::parse_with_recovery(tokens);
    if !parse_errors.is_empty() {
        // Há erros de parse — publica diagnósticos de parse sem continuar
        // para resolve/infer. Os items válidos no module não são suficientes
        // para um resolve confiável (símbolos referenciados podem faltar).
        return Err(parse_errors.into_iter().map(FrontendBatch::Parse).collect());
    }

    // 3. Resolve (prelude + módulo do usuário)
    let prelude = load_stdlib().map_err(|e| vec![FrontendBatch::Resolve(e)])?;
    let user = kata_resolution::resolve(&module).map_err(|e| vec![FrontendBatch::Resolve(e)])?;
    let mut resolved = kata_resolution::merge_two(prelude, user);

    // 3a. Imports multi-arquivo (se file_path disponível)
    if let Some(file) = file_path {
        let imports = load_module_imports(file, &module);
        merge_imports(&mut resolved, &imports);
    }

    // 4. Infer (typeck + dispatch)
    let typed = kata_inference::infer_module(&module, &resolved)
        .map_err(|e| vec![FrontendBatch::Infer(e)])?;

    Ok(FrontendResult {
        module,
        resolved,
        typed,
    })
}

/// Carrega módulos importados por um arquivo.
///
/// Cria um `ModuleLoader` com search paths = diretório do arquivo + stdlib.
/// Retorna a lista de `ImportedModule` (vazia se não há imports ou erro).
fn load_module_imports(file: &str, module: &Module) -> Vec<ImportedModule> {
    let entry_dir = Path::new(file)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    // stdlib relativa ao crate (mesmo padrão do driver)
    let stdlib_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../stdlib")
        .canonicalize()
        .unwrap_or_else(|_| Path::new("../../stdlib").to_path_buf());

    let search_paths = vec![entry_dir.clone(), stdlib_dir];
    let mut loader = ModuleLoader::new(search_paths);
    loader.load_imports(module, &entry_dir).unwrap_or_default()
}

/// Carrega a stdlib (core → core_internals) via `ModuleLoader`, substituindo
/// Retorna o `ResolvedModule` não-filtrado.
fn load_stdlib() -> Result<ResolvedModule, Vec<kata_resolution::ResolveError>> {
    let mut loader = ModuleLoader::new(Vec::new());
    let stdlib = loader
        .load(&["stdlib".into(), "core".into()], Path::new("."))
        .map_err(|e| match e {
            kata_resolution::LoadError::Resolve(errors) => errors,
            other => vec![kata_resolution::ResolveError::UnknownFfi {
                name: format!("erro ao carregar stdlib: {other}"),
            }],
        })?;
    Ok((*stdlib).clone())
}
