//! Merge de módulos importados no `ResolvedModule`.
//!
//! Para cada `ImportedModule` carregado pelo `ModuleLoader`:
//! - `Selective { items }`: traz itens nomeados para o escopo direto (sem prefixo).
//! - `WholeModule { prefix }`: registra cada item exportado com nome qualificado
//!   `prefix.item` nas signatures/functions/actions.
//! - `WholeModuleAliased { alias }`: mesmo que WholeModule mas com prefixo alias.

use std::path::Path;

use kata_ast::Item;
use kata_comptime::run_comptime_pass;
use kata_inference::{TypedExpr, TypedExprKind, infer_module};
use kata_resolution::{ImportKind, ImportedModule, ModuleLoader, ResolvedModule};

use crate::IntoReport;

/// Mergeia módulos importados no ResolvedModule (prelude + user já mergeados).
///
/// Para cada `ImportedModule`:
/// - `Selective { items }`: traz itens nomeados para o escopo direto (sem prefixo).
/// - `WholeModule { prefix }`: registra cada item exportado com nome qualificado
///   `prefix.item` nas signatures/functions/actions. O inference resolve
///   `mod.fn` como `DotAccess { Ident("mod"), Field("fn") }` procurando
///   `mod.fn` no DispatchTable.
/// - `WholeModuleAliased { alias }`: mesmo que WholeModule mas com prefixo alias.
pub(crate) fn merge_imports(merged: &mut ResolvedModule, imports: &[ImportedModule]) {
    for imported in imports {
        let origin = &imported.module_name;
        match &imported.import_kind {
            ImportKind::Selective { items } => {
                // Import seletivo: trazer itens nomeados para o escopo direto.
                // Cada item pode ter alias: `dobrar as d` → registra como `d`.
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
                // Copiar registries do módulo importado (transitivo para
                // interfaces/structs/enums referenciados pelos itens).
                merge_registries(merged, &imported.resolved);
            }
            ImportKind::WholeModule { prefix } => {
                // Módulo inteiro: registrar cada item exportado com nome
                // qualificado `prefix.item`. O inference resolve DotAccess
                // { Ident("mod"), Field("fn") } procurando `mod.fn` no
                // DispatchTable.
                register_qualified(merged, prefix, &imported.resolved);
                // Copiar tipos com nome qualificado `prefix.Type`
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
                // Copiar tipos com nome qualificado `alias.Type`
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
/// importado para o módulo merged. Não sobrescreve entradas existentes
/// (tipos locais têm prioridade).
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
    // Mescla DirectiveRegistry — diretivas importadas ficam disponíveis
    // para o módulo importador. Overloads por (when, on) coexistem.
    let _merge_errors = merged
        .directive_registry
        .merge(imported.directive_registry.clone());
}

/// Registra itens de um módulo importado com nome qualificado `prefix.item`
/// e também no escopo direto (não-qualificado).
///
/// Renomeia signatures, functions e actions com o prefixo qualificado.
/// Isso garante consistência em todos os passes:
/// - DispatchTable: signature.name = "mod.fn"
/// - TypedFunction: func.name = "mod.fn" (infer_module usa func_def.name)
/// - symbol_table/kata_ids: chave = ("mod.fn", params, ret)
/// - tree_shaking: fn_names e reached_fns usam "mod.fn"
///
/// Além da forma qualificada, traz signatures e functions para o escopo
/// direto (não-qualificado) para que operadores e métodos de interface
/// importados sejam encontrados pelo dispatch. Por exemplo, `import complex`
/// traz `+ :: Complex Complex => Complex` como `+` (escopo direto) além de
/// `complex.+` (acesso qualificado). O dispatch por tipos escolhe o overload
/// correto entre prelude e importados.
///
/// Colisões: se já existe uma signature com mesmo nome E mesmos tipos
/// (params + return) no merged, não duplica. Se existe com tipos diferentes,
/// é um overload legítimo — ambos coexistem.
fn register_qualified(merged: &mut ResolvedModule, prefix: &str, resolved: &ResolvedModule) {
    for sig in &resolved.signatures {
        // Forma qualificada: prefix.sig
        let qual_name = format!("{prefix}.{}", sig.name);
        if !merged.signatures.iter().any(|s| s.name == qual_name) {
            let mut qual_sig = sig.clone();
            qual_sig.name = qual_name;
            merged.signatures.push(qual_sig);
        }
        // Forma não-qualificada: sig (escopo direto)
        // Só insere se não existe signature idêntica (nome + tipos).
        // Overloads de mesmo nome com tipos diferentes coexistem.
        let dup = merged.signatures.iter().any(|s| {
            s.name == sig.name
                && s.param_types == sig.param_types
                && s.return_type == sig.return_type
        });
        if !dup {
            merged.signatures.push(sig.clone());
        }
    }
    for func in &resolved.functions {
        let qual_name = format!("{prefix}.{}", func.name);
        if !merged.functions.iter().any(|f| f.name == qual_name) {
            let mut qual_func = func.clone();
            qual_func.name = qual_name;
            merged.functions.push(qual_func);
        }
        // Forma não-qualificada — mesmo critério de duplicata.
        let dup = merged.functions.iter().any(|f| {
            f.name == func.name
                && f.param_types == func.param_types
                && f.return_type == func.return_type
        });
        if !dup {
            merged.functions.push(func.clone());
        }
    }
    for action in &resolved.actions {
        let qual_name = format!("{prefix}.{}", action.name);
        if !merged.actions.iter().any(|a| a.name == qual_name) {
            let mut qual_action = action.clone();
            qual_action.name = qual_name;
            merged.actions.push(qual_action);
        }
        // Forma não-qualificada — actions não têm overloads, checa só nome.
        if !merged.actions.iter().any(|a| a.name == action.name) {
            merged.actions.push(action.clone());
        }
    }
}

/// Carrega módulos importados por um arquivo.
///
/// Cria um `ModuleLoader` com search paths = diretório do arquivo + stdlib.
/// Retorna a lista de `ImportedModule` (vazia se não há imports).
/// O prelude (core) é injetado separadamente pelo caller via load_prelude.
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

    let search_paths = vec![entry_dir, stdlib_dir];
    let mut loader = ModuleLoader::new(search_paths);
    loader.load_imports(module).map_err(|e| {
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

/// Extrai o `DirectiveRegistry` dos módulos importados, para ser usado
/// como base no `resolve_with_imports` do módulo do usuário.
///
/// Isto permite que `@trace_enter` referencie uma diretiva definida num
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
        let typed = run_comptime_pass(typed, &imported.resolved_unfiltered.enum_registry)
            .map_err(|e| e.into_report_with_source("", None))?;

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
