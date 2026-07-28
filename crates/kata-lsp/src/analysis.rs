//! Front-end pipeline reusável: lex → parse → resolve → infer (sem codegen).
//!
//! O LSP consome apenas o front-end — sem codegen, sem runtime, sem side-effects.
//! O `FrontendResult` carrega tudo que tooling precisa: AST, ResolvedModule, TAST.
//!
//! Imports multi-arquivo são resolvidos via `ModuleLoader` de `kata-resolution`
//! (API pública). O merge de imports é reimplementado aqui (~80 linhas) porque
//! `kata-driver::imports` é `pub(crate)` e depender de `kata-driver` puxaria
//! codegen + runtime como deps transitivas.

use std::path::Path;

use kata_ast::Module;
use kata_inference::TypedModule;
use kata_resolution::{ImportKind, ImportedModule, ModuleLoader, ResolvedModule};

/// Resultado do front-end — tudo que o LSP precisa.
pub(crate) struct FrontendResult {
    #[allow(dead_code, reason = "usado por go-to-def futuro")]
    pub module: Module,
    #[allow(dead_code, reason = "usado por go-to-def futuro")]
    pub resolved: ResolvedModule,
    pub typed: TypedModule,
}

/// Erros do front-end coletados em um único enum.
pub(crate) enum FrontendError {
    Lex(kata_diagnostics::FrontendError),
    Parse(kata_diagnostics::FrontendError),
    Resolve(Vec<kata_resolution::ResolveError>),
    Infer(kata_diagnostics::MiddleError),
}

/// Roda lex → parse → resolve(+imports) → infer (sem codegen).
pub(crate) fn run_frontend(
    source: &str,
    file_path: Option<&str>,
) -> Result<FrontendResult, Vec<FrontendError>> {
    // 1. Lex
    let tokens = kata_lexer::lex(source).map_err(|e| vec![FrontendError::Lex(e)])?;

    // 2. Parse
    let module = kata_parser::parse(tokens).map_err(|e| vec![FrontendError::Parse(e)])?;

    // 3. Resolve (prelude + módulo do usuário)
    let prelude = kata_resolution::load_prelude().map_err(|e| vec![FrontendError::Resolve(e)])?;
    let user = kata_resolution::resolve(&module).map_err(|e| vec![FrontendError::Resolve(e)])?;
    let mut resolved = kata_resolution::merge_two(prelude, user);

    // 3a. Imports multi-arquivo (se file_path disponível)
    if let Some(file) = file_path {
        let imports = load_module_imports(file, &module);
        merge_imports(&mut resolved, &imports);
    }

    // 4. Infer (typeck + dispatch)
    let typed = kata_inference::infer_module(&module, &resolved)
        .map_err(|e| vec![FrontendError::Infer(e)])?;

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

    let search_paths = vec![entry_dir, stdlib_dir];
    let mut loader = ModuleLoader::new(search_paths);
    loader.load_imports(module).unwrap_or_default()
}

/// Mergeia imports no `ResolvedModule` (reimplementação de
/// `kata_driver::imports::merge_imports`, que é `pub(crate)`).
fn merge_imports(merged: &mut ResolvedModule, imports: &[ImportedModule]) {
    for imported in imports {
        let origin = &imported.module_name;
        match &imported.import_kind {
            ImportKind::Selective { items } => {
                for imp_item in items {
                    let target_name = imp_item.alias.as_ref().unwrap_or(&imp_item.name);
                    // Signatures
                    if let Some(sig) = imported
                        .resolved
                        .signatures
                        .iter()
                        .find(|s| s.name == imp_item.name)
                        && !merged.signatures.iter().any(|s| s.name == *target_name)
                    {
                        let mut renamed = sig.clone();
                        renamed.name = target_name.clone();
                        merged.signatures.push(renamed);
                    }
                    // Functions
                    if let Some(func) = imported
                        .resolved
                        .functions
                        .iter()
                        .find(|f| f.name == imp_item.name)
                        && !merged.functions.iter().any(|f| f.name == *target_name)
                    {
                        let mut renamed = func.clone();
                        renamed.name = target_name.clone();
                        merged.functions.push(renamed);
                    }
                    // Actions
                    if let Some(action) = imported
                        .resolved
                        .actions
                        .iter()
                        .find(|a| a.name == imp_item.name)
                        && !merged.actions.iter().any(|a| a.name == *target_name)
                    {
                        let mut renamed = action.clone();
                        renamed.name = target_name.clone();
                        merged.actions.push(renamed);
                    }
                    // TypeEnv: copiar binding do tipo importado
                    if let Some(binding) = imported.resolved.type_env.lookup_binding(&imp_item.name)
                    {
                        merged
                            .type_env
                            .define(target_name, binding.ty.clone(), origin);
                    }
                }
                merge_registries(merged, &imported.resolved);
            }
            ImportKind::WholeModule { prefix } => {
                register_qualified(merged, prefix, &imported.resolved);
                for (name, binding) in imported.resolved.type_env.local_bindings_full() {
                    let qual_name = format!("{prefix}.{name}");
                    merged
                        .type_env
                        .define(&qual_name, binding.ty.clone(), origin);
                }
                merge_registries(merged, &imported.resolved);
            }
            ImportKind::WholeModuleAliased { alias } => {
                register_qualified(merged, alias, &imported.resolved);
                for (name, binding) in imported.resolved.type_env.local_bindings_full() {
                    let qual_name = format!("{alias}.{name}");
                    merged
                        .type_env
                        .define(&qual_name, binding.ty.clone(), origin);
                }
                merge_registries(merged, &imported.resolved);
            }
        }
    }
}

/// Mergeia registries (enum, struct, interface, refines) do módulo
/// importado para o módulo merged. Não sobrescreve entradas existentes.
fn merge_registries(merged: &mut ResolvedModule, imported: &ResolvedModule) {
    merged.enum_registry.merge(imported.enum_registry.clone());
    merged
        .struct_registry
        .merge(imported.struct_registry.clone());
    merged
        .interface_registry
        .merge(imported.interface_registry.clone());
    merged
        .refines_registry
        .merge(imported.refines_registry.clone());
}

/// Registra itens de um módulo importado com nome qualificado `prefix.item`.
fn register_qualified(merged: &mut ResolvedModule, prefix: &str, resolved: &ResolvedModule) {
    for sig in &resolved.signatures {
        let qual_name = format!("{prefix}.{}", sig.name);
        if !merged.signatures.iter().any(|s| s.name == qual_name) {
            let mut qual_sig = sig.clone();
            qual_sig.name = qual_name;
            merged.signatures.push(qual_sig);
        }
    }
    for func in &resolved.functions {
        let qual_name = format!("{prefix}.{}", func.name);
        if !merged.functions.iter().any(|f| f.name == qual_name) {
            let mut qual_func = func.clone();
            qual_func.name = qual_name;
            merged.functions.push(qual_func);
        }
    }
    for action in &resolved.actions {
        let qual_name = format!("{prefix}.{}", action.name);
        if !merged.actions.iter().any(|a| a.name == qual_name) {
            let mut qual_action = action.clone();
            qual_action.name = qual_name;
            merged.actions.push(qual_action);
        }
    }
}
