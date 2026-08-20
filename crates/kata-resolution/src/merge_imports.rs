//! Merge de módulos importados no `ResolvedModule`.
//!
//! Para cada `ImportedModule`:
//! - `Selective { items }`: traz itens nomeados para o escopo direto (sem prefixo).
//! - `WholeModule { prefix }`: registra cada item exportado com nome qualificado
//!   `prefix.item` nas signatures/functions/actions.
//! - `WholeModuleAliased { alias }`: mesmo que WholeModule mas com prefixo alias.

use crate::{ImportKind, ImportedModule, ResolvedModule};

/// Mergeia módulos importados no ResolvedModule (prelude + user já mergeados).
///
/// Para cada `ImportedModule`:
/// - `Selective { items }`: traz itens nomeados para o escopo direto (sem prefixo).
/// - `WholeModule { prefix }`: registra cada item exportado com nome qualificado
///   `prefix.item` nas signatures/functions/actions. O inference resolve
///   `mod.fn` como `DotAccess { Ident("mod"), Field("fn") }` procurando
///   `mod.fn` no DispatchTable.
/// - `WholeModuleAliased { alias }`: mesmo que WholeModule mas com prefixo alias.
pub fn merge_imports(merged: &mut ResolvedModule, imports: &[ImportedModule]) {
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
            s.name == sig.name && s.param_types == sig.param_types && s.return_type == sig.return_type
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
            f.name == func.name && f.param_types == func.param_types && f.return_type == func.return_type
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