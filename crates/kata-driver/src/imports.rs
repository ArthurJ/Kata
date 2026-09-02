//! Carregamento e merge de módulos importados (driver-side helpers).
//!
//! As funções de merge (`merge_imports`, `merge_registries`,
//! `register_qualified`) foram movidas para `kata-resolution`.
//! Este módulo mantém apenas os helpers do driver.

use std::path::Path;

use kata_ast::Item;
use kata_comptime::run_comptime_pass;
use kata_inference::{TypedExpr, TypedExprKind, infer_module};
use kata_resolution::{ImportedModule, ModuleLoader};

use crate::IntoReport;

/// Carrega módulos importados por um arquivo.
///
/// Cria um `ModuleLoader` com search paths = diretório do arquivo + stdlib.
/// Retorna a lista de `ImportedModule` (vazia se não há imports).
/// O prelude (core) é injetado separadamente pelo caller via load_stdlib.
pub(crate) fn load_module_imports(
    file: &str,
    module: &kata_ast::Module,
) -> miette::Result<Vec<ImportedModule>> {
    let entry_dir = Path::new(file)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let stdlib_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../stdlib")
        .canonicalize()
        .unwrap_or_else(|_| Path::new("../../stdlib").to_path_buf());

    let search_paths = vec![entry_dir.clone(), stdlib_dir];
    let mut loader = ModuleLoader::new(search_paths);
    loader.load_imports(module, &entry_dir).map_err(|e| {
        // LoadError carrega tipos estruturados (FrontendError, Vec<ResolveError>).
        // Para source context completo, precisaríamos do source code do
        // módulo importado — o ModuleLoader não o expõe. Por ora, extrai
        // o erro interno para ter código miette + span (sem linha de código).
        match e {
            kata_resolution::LoadError::Lex(inner) | kata_resolution::LoadError::Parse(inner) => {
                inner.into_report_with_source("", None)
            }
            kata_resolution::LoadError::Resolve(errors) => {
                // ResolveError não tem #[label]/span — apenas código + mensagem.
                // Reporta o primeiro erro como Report principal.
                if let Some(first) = errors.first() {
                    first.clone().into_report_with_source("", None)
                } else {
                    miette::Report::msg("erro de resolução ao carregar módulo (sem detalhes)")
                }
            }
            other => miette::Report::msg(format!("erro ao carregar imports: {other}")),
        }
    })
}

/// Carrega módulos importados no contexto do REPL.
///
/// Diferente de `load_module_imports` (que usa o diretório do arquivo como
/// search path), esta função usa o diretório atual (`.`) + stdlib. O REPL
/// não tem arquivo-base, então `.` é o ponto de partida natural.
pub(crate) fn load_repl_imports(module: &kata_ast::Module) -> miette::Result<Vec<ImportedModule>> {
    let stdlib_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../stdlib")
        .canonicalize()
        .unwrap_or_else(|_| Path::new("../../stdlib").to_path_buf());

    let search_paths = vec![Path::new(".").to_path_buf(), stdlib_dir];
    let mut loader = ModuleLoader::new(search_paths);
    loader
        .load_imports(module, Path::new("."))
        .map_err(|e| match e {
            kata_resolution::LoadError::Lex(inner) | kata_resolution::LoadError::Parse(inner) => {
                inner.into_report_with_source("", None)
            }
            kata_resolution::LoadError::Resolve(errors) => {
                if let Some(first) = errors.first() {
                    first.clone().into_report_with_source("", None)
                } else {
                    miette::Report::msg("erro de resolução ao carregar módulo (sem detalhes)")
                }
            }
            other => miette::Report::msg(format!("erro ao carregar imports: {other}")),
        })
}

/// Extrai o `DirectiveRegistry` dos módulos importados, para ser usado
/// como base no `resolve_with_imports` do módulo do usuário.
///
/// Isto permite que `@log` referencie uma diretiva definida num
/// módulo importado — o registry já contém a diretiva quando a validação
/// de `@nome` roda no Pass 1 do resolve.
pub(crate) fn collect_imported_directives(
    imports: &[ImportedModule],
) -> kata_resolution::DirectiveRegistry {
    let mut registry = kata_resolution::DirectiveRegistry::new();
    for imported in imports {
        let errors = registry.merge(imported.resolved.directive_registry.clone());
        // Erros de merge (diretivas duplicadas entre módulos importados)
        // são silenciados aqui — serão detectados no merge final.
        let _ = errors;
    }
    registry
}

/// Uma constant exportada por um módulo importado, já avaliada pelo comptime pass.
/// O valor é um `TypedExpr` (literal escalar ou `HeapSnapshot` para tipos complexos).
pub(crate) struct ImportedConstant {
    pub name: String,
    pub value: TypedExpr,
}

/// Roda inference + comptime pass recursivamente em cada módulo importado
/// e extrai as constants exportadas (avaliadas para literal/HeapSnapshot).
///
/// O pipeline do importado usa `resolved_unfiltered` (com todos os itens,
/// incluindo helpers não-exportados) para que o inference tenha acesso a
/// funções auxiliares que as constants podem referenciar. Só as constants
/// listadas em `export` do módulo importado são extraídas.
///
/// Retorna um mapa: nome_da_constant → ImportedConstant.
pub(crate) fn evaluate_imported_constants(
    imports: &[ImportedModule],
) -> miette::Result<Vec<ImportedConstant>> {
    let mut result = Vec::new();
    for imported in imports {
        // O módulo importado pode não ter entry point (só constants + exports).
        // infer_module exige um entry point, então injetamos um IntLit(0)
        // sintético no final do AST se não houver EntryExpr.
        let module = if has_entry_expr(&imported.module_ast) {
            imported.module_ast.clone()
        } else {
            inject_synthetic_entry(imported.module_ast.clone())
        };

        // Rodar inference no módulo importado (não-filtrado).
        let typed = infer_module(&module, &imported.resolved_unfiltered)
            .map_err(|e| e.into_report_with_source("", None))?;

        // Rodar comptime pass para avaliar constants.
        // O comptime Runtime é efêmero — se `set_recursion_limit` estiver
        // num módulo importado, o limite não propaga para o módulo
        // importador (caso edge: raro e sem caso de uso claro).
        let comptime_rt = Box::new(kata_rt::Runtime::new());
        let comptime_rt_ptr = Box::into_raw(comptime_rt) as i64;
        let typed = run_comptime_pass(typed, &imported.resolved_unfiltered.enum_registry, comptime_rt_ptr)
            .map_err(|e| e.into_report_with_source("", None))?;
        // Droppar o comptime Runtime — valores já foram consumidos.
        unsafe { drop(Box::from_raw(comptime_rt_ptr as *mut kata_rt::Runtime)) };

        // Coletar nomes exportados do AST do módulo importado.
        let exported_names: std::collections::HashSet<String> = imported
            .module_ast
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

        // Se não há export decl, todas as constants são exportadas (módulo aberto).
        let has_export = exported_names.is_empty();
        let _ = has_export; // módulo aberto → todas exportadas

        // Extrair constants avaliadas.
        for binding in &typed.constants {
            if let TypedExprKind::ConstantBinding { name, value } = &binding.node.kind {
                // Só exportadas (ou todas se módulo aberto).
                if has_export || exported_names.contains(name) || name == "_" {
                    result.push(ImportedConstant {
                        name: name.clone(),
                        value: value.node.clone(),
                    });
                }
            }
        }
    }
    Ok(result)
}

/// Verifica se o módulo tem pelo menos um `Item::EntryExpr`.
fn has_entry_expr(module: &kata_ast::Module) -> bool {
    module
        .items
        .iter()
        .any(|item| matches!(item.node, kata_ast::Item::EntryExpr(_)))
}

/// Injeta um `IntLit(0)` sintético como `EntryExpr` no final do módulo.
/// Necessário porque `infer_module` exige um entry point, mas módulos
/// exportadores de constants podem não ter um.
fn inject_synthetic_entry(mut module: kata_ast::Module) -> kata_ast::Module {
    let span = kata_ast::Span::zero();
    let zero = kata_ast::Spanned::new(
        kata_ast::Expr::IntLit {
            text: "0".to_string(),
        },
        span,
    );
    module.items.push(kata_ast::Spanned::new(
        kata_ast::Item::EntryExpr(zero),
        span,
    ));
    module
}
