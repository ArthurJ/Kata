//! Expansão de famílias polimórficas (`data (IFACE, ...) as Fam`).
//!
//! Contém as três operações relacionadas a famílias polimórficas que rodam
//! após o merge do prelude:
//!
//! - `extend_families_for_implementors`: registra instâncias faltantes
//!   (`Fam::T`) para implementors tardios do usuário.
//! - `reinstantiate_family_params`: re-instancia `Family(name)` →
//!   `Instance(name, concrete)` em signatures/functiondefs que não puderam
//!   ser instanciadas no pass0.
//! - `expand_family_signatures`: expande signatures FFI com `Family` nos
//!   params em uma versão concreta por instância (chamado pelo inference).

use std::collections::HashMap;

use kata_ast::Span;
use kata_core::{InterfaceRegistry, StructRegistry, TypeGraph};
use kata_core::{PrimTy, StructKey, Ty};

use crate::pass0::instantiate_family_for_concrete;
use crate::{FunctionDef, RefinedDeclInfo, Signature};

/// Re-instancia `Family(name)` → `Instance(name, concrete)` em signatures e
/// functiondefs que não puderam ser instanciadas no pass0 porque a instância
/// da família ainda não havia sido registrada.
///
/// Após `extend_families_for_implementors`, as instâncias faltantes existem.
/// Para cada `(type_name, iface_name)` nos impls, lista os métodos da interface
/// e re-aplica `instantiate_family_for_concrete` com `concrete = type_name`
/// nas signatures/functiondefs cujo nome é um método dessa interface e que
/// ainda contêm `Family` nos param_types.
pub(crate) fn reinstantiate_family_params(
    signatures: &mut [Signature],
    functions: &mut [FunctionDef],
    interface_registry: &InterfaceRegistry,
    struct_registry: &StructRegistry,
) {
    // Para cada impl, coletar (type_name, method_names da interface).
    let impls: Vec<(String, String)> = interface_registry
        .impls_view()
        .iter()
        .map(|e| (e.type_name.clone(), e.interface_name.clone()))
        .collect();

    // Construir mapa: method_name → set de (type_name) que implementam
    // uma interface contendo esse método.
    // Para cada (type_name, iface_name), listar métodos da iface
    // (incluindo herdados de supertraits — ex: NUM herda / de FIELD).
    let mut method_to_types: HashMap<String, Vec<String>> = HashMap::new();
    for (type_name, iface_name) in &impls {
        let methods = interface_registry.all_method_names(iface_name);
        for method_name in methods {
            method_to_types
                .entry(method_name)
                .or_default()
                .push(type_name.clone());
        }
    }

    // Filtro de signatures fantasma: mapa method_name → set de Self-types
    // que TÊM uma signature/FunctionDef definida (não apenas implementam a
    // interface). Isto evita re-instanciar `mod :: Self NonZero => Self`
    // para `Complex` quando `Complex` não define `mod` (apenas implementa
    // NUM que contém `mod` como método).
    let mut method_definers: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for sig in signatures.iter() {
        if let Some(self_name) = sig.param_types.first().and_then(|ty| match ty {
            Ty::Struct(StructKey::Plain(name)) => Some(name.clone()),
            Ty::Prim(PrimTy::Int) => Some("Int".to_string()),
            Ty::Prim(PrimTy::Float) => Some("Float".to_string()),
            Ty::Prim(PrimTy::Rational) => Some("Rational".to_string()),
            Ty::Prim(PrimTy::Text) => Some("Text".to_string()),
            _ => None,
        }) {
            method_definers
                .entry(sig.name.clone())
                .or_default()
                .insert(self_name);
        }
    }
    for func in functions.iter() {
        if let Some(self_name) = func.param_types.first().and_then(|ty| match ty {
            Ty::Struct(StructKey::Plain(name)) => Some(name.clone()),
            Ty::Prim(PrimTy::Int) => Some("Int".to_string()),
            Ty::Prim(PrimTy::Float) => Some("Float".to_string()),
            Ty::Prim(PrimTy::Rational) => Some("Rational".to_string()),
            Ty::Prim(PrimTy::Text) => Some("Text".to_string()),
            _ => None,
        }) {
            method_definers
                .entry(func.name.clone())
                .or_default()
                .insert(self_name);
        }
    }

    // Re-instanciar signatures.
    for sig in signatures.iter_mut() {
        // Só processar se tem Family OU Plain-de-família nos params.
        // Plain("NonZero") aparece quando o pass0 do usuário resolveu "NonZero"
        // sem saber que é família (o struct_registry do usuário não tem o
        // prelude merged). Após merge_two, o struct_registry merged sabe.
        let needs_reinst = sig.param_types.iter().any(|ty| match ty {
            Ty::Struct(StructKey::Family(_)) => true,
            Ty::Struct(StructKey::Plain(name)) => struct_registry.is_family(name),
            _ => false,
        });
        if !needs_reinst {
            continue;
        }
        // Determinar o concrete_type: é o Self da interface — o tipo que
        // aparece como Struct(Plain(name)) no primeiro param (convenção de
        // métodos de interface: Self é o receptor/primeiro param).
        // Só re-instanciar se o Self é um tipo que implementa a interface
        // e está na lista de candidates.
        let self_type_name = sig.param_types.first().and_then(|ty| match ty {
            Ty::Struct(StructKey::Plain(name)) => Some(name.clone()),
            Ty::Prim(_) => {
                // Primitivos: Int, Float, etc. Já foram instanciados no
                // pass0 — não deveriam ter Family aqui. Pular.
                None
            }
            _ => None,
        });

        let Some(concrete_type) = self_type_name else {
            continue;
        };

        // Verificar que este concrete_type está nos candidates (implementa
        // uma interface com este método).
        let is_candidate = method_to_types
            .get(&sig.name)
            .map(|types| types.contains(&concrete_type))
            .unwrap_or(false);
        if !is_candidate {
            continue;
        }

        // Filtro de signature fantasma: só re-instanciar se o concrete_type
        // DEFINE o método (tem signature própria), não apenas se implementa
        // a interface. Evita criar `mod :: Complex NonZero::Complex =>
        // Complex` quando Complex não define `mod`.
        let defines_method = method_definers
            .get(&sig.name)
            .map(|set| set.contains(&concrete_type))
            .unwrap_or(false);
        if !defines_method {
            continue;
        }

        let new_params: Vec<Ty> = sig
            .param_types
            .iter()
            .map(|ty| instantiate_family_for_concrete(ty, &concrete_type, struct_registry))
            .collect();
        if new_params != sig.param_types {
            sig.param_types = new_params;
        }
    }

    // Re-instanciar functiondefs.
    for func in functions.iter_mut() {
        let needs_reinst = func.param_types.iter().any(|ty| match ty {
            Ty::Struct(StructKey::Family(_)) => true,
            Ty::Struct(StructKey::Plain(name)) => struct_registry.is_family(name),
            _ => false,
        });
        if !needs_reinst {
            continue;
        }
        // Mesma lógica: Self = primeiro param (Struct(Plain(name))).
        let self_type_name = func.param_types.first().and_then(|ty| match ty {
            Ty::Struct(StructKey::Plain(name)) => Some(name.clone()),
            _ => None,
        });
        let Some(concrete_type) = self_type_name else {
            continue;
        };
        let is_candidate = method_to_types
            .get(&func.name)
            .map(|types| types.contains(&concrete_type))
            .unwrap_or(false);
        if !is_candidate {
            continue;
        }
        // Mesmo filtro de signature fantasma para functiondefs.
        let defines_method = method_definers
            .get(&func.name)
            .map(|set| set.contains(&concrete_type))
            .unwrap_or(false);
        if !defines_method {
            continue;
        }
        let new_params: Vec<Ty> = func
            .param_types
            .iter()
            .map(|ty| instantiate_family_for_concrete(ty, &concrete_type, struct_registry))
            .collect();
        if new_params != func.param_types {
            func.param_types = new_params;
            let new_ret =
                instantiate_family_for_concrete(&func.return_type, &concrete_type, struct_registry);
            func.return_type = new_ret;
        }
    }
}

/// Estende famílias polimórficas com instâncias faltantes para implementors
/// tardios.
///
/// Após `merge_two`, o `interface_registry` contém todos os `implements` do
/// prelude + usuário, e o `struct_registry` contém todas as famílias. Para
/// cada `T implements IFACE`, se existe uma família `data (IFACE, ...) as Fam`
/// que não tem `Fam::T` registrada, registra a instância faltante + o
/// `RefinedDeclInfo` correspondente para o inference sintetizar o construtor.
///
/// Origin da instância: a origin da família (resolvida via
/// `struct_registry.resolve_origin(family)`), não a origin do implementor.
/// Isto porque `get_instance(family, concrete)` resolve origin pelo nome
/// da família — se registrássemos com origin do usuário, a busca falharia.
pub(crate) fn extend_families_for_implementors(
    struct_registry: &mut StructRegistry,
    refined_decls: &mut Vec<RefinedDeclInfo>,
    interface_registry: &InterfaceRegistry,
    type_graph: &mut TypeGraph,
) {
    // Coletar todos os (type_name, iface_name, span, allows_incomplete) dos impls.
    // O span é necessário para produzir o diagnóstico
    // `FamilyExtensionInvalid` no inference quando um predicado falha.
    // allows_incomplete indica se o impl tem #!allow type.incomplete_interface.
    let impls: Vec<(String, String, Span, bool)> = interface_registry
        .impls_view()
        .iter()
        .map(|e| (e.type_name.clone(), e.interface_name.clone(), e.span, e.allows_incomplete))
        .collect();

    for (type_name, iface_name, impl_span, allows_incomplete) in &impls {
        // Encontrar famílias sobre esta interface.
        let families = struct_registry.families_over_iface(iface_name);
        for family in &families {
            // Pular se a instância já existe (idempotência).
            if struct_registry.has_instance(family, type_name) {
                continue;
            }

            // Resolver a origin da família para registrar a instância.
            let Some(family_origin) = struct_registry
                .resolve_origin(family)
                .map(|s| s.to_string())
            else {
                continue;
            };

            // Obter os predicados de uma instância existente da mesma família
            // para derivar os pred_names da nova instância.
            // Estrutura: __pred_{family}_{concrete}_{idx}
            let num_preds = struct_registry
                .all_instances(family)
                .first()
                .and_then(|(_, info)| info.predicates.as_ref())
                .map(|preds| preds.len())
                .unwrap_or(0);

            let pred_names: Vec<String> = (0..num_preds)
                .map(|i| format!("__pred_{family}_{type_name}_{i}"))
                .collect();

            // Determinar o base_ty da instância (tipo concreto).
            let instance_base = match type_name.as_str() {
                "Int" => Ty::Prim(PrimTy::Int),
                "Float" => Ty::Prim(PrimTy::Float),
                "Rational" => Ty::Prim(PrimTy::Rational),
                "Text" => Ty::Prim(PrimTy::Text),
                _ => Ty::Struct(StructKey::Plain(type_name.clone())),
            };

            // Registrar a instância no struct_registry.
            struct_registry.register_refined_instance(
                &family_origin,
                family,
                type_name,
                pred_names,
            );

            // Sincronizar o TypeGraph: adicionar a instância ao nó Family.
            type_graph.add_family_instance(family, type_name);

            // Adicionar RefinedDeclInfo para o inference sintetizar o construtor.
            // Os predicados são os mesmos da família (extraídos de uma
            // instância existente via refined_decls).
            let template_found = refined_decls
                .iter()
                .find(|rd| rd.name == *family && rd.lazy_type_param.is_none());
            if let Some(template) = template_found {
                refined_decls.push(RefinedDeclInfo {
                    name: family.clone(),
                    base_ty: instance_base,
                    predicates: template.predicates.clone(),
                    lazy_type_param: None,
                    extension_impl: Some((type_name.clone(), iface_name.clone(), *impl_span)),
                    allows_incomplete: *allows_incomplete,
                });
            }
        }
    }
}

/// Expande signatures e functions que usam `Family("FamilyName")` de uma
/// família polimórfica em múltiplas versões concretas, uma por instância.
///
/// `Family("NonZero")` é produzido por `resolve_type_expr` quando o nome
/// é uma família polimórfica registrada. A expansão substitui cada
/// `Family("NonZero")` por `Instance("NonZero","Int")`,
/// `Instance("NonZero","Float")`, etc. O tree-shaking remove as
/// instâncias sem chamador.
pub fn expand_family_signatures(
    signatures: &mut Vec<Signature>,
    _struct_registry: &StructRegistry,
) {
    // Para cada signature, verificar se algum param é Family.
    // Se sim, expandir em instâncias concretas.
    //
    // Filtro de signatures fantasma: para cada instância (concrete, _) da
    // família, só criar a signature expandida se o tipo base (concrete)
    // já tem uma overload do mesmo método com Self = concrete nas
    // signatures originais. Isto evita criar signatures fantasma como
    // `/ :: Complex NonZero::Complex => Complex` quando Complex não
    // define `/` — a signature não teria FFI symbol nem corpo e
    // confunde o dispatch durante a síntese de predicados.
    let mut new_sigs = Vec::new();

    // Snapshot dos Self-types por método ANTES da expansão.
    // method_selfs["/"] = {"Int", "Float", "Rational"} (não inclui "Complex"
    // porque Complex não define `/`).
    let mut method_selfs: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for sig in signatures.iter() {
        if let Some(self_name) = sig.param_types.first().and_then(|ty| match ty {
            Ty::Struct(StructKey::Plain(name)) => Some(name.clone()),
            Ty::Prim(PrimTy::Int) => Some("Int".to_string()),
            Ty::Prim(PrimTy::Float) => Some("Float".to_string()),
            Ty::Prim(PrimTy::Rational) => Some("Rational".to_string()),
            Ty::Prim(PrimTy::Text) => Some("Text".to_string()),
            _ => None,
        }) {
            method_selfs
                .entry(sig.name.clone())
                .or_default()
                .insert(self_name);
        }
    }

    for sig in signatures.iter() {
        // Coletar posições dos params que são Family.
        let family_positions: Vec<(usize, String)> = sig
            .param_types
            .iter()
            .enumerate()
            .filter_map(|(i, ty)| {
                if let Ty::Struct(StructKey::Family(name)) = ty {
                    Some((i, name.clone()))
                } else {
                    None
                }
            })
            .collect();

        if family_positions.is_empty() {
            // Sem famílias — signature fica como está.
            continue;
        }

        // Gerar combinações: produto cartesiano das instâncias de cada família.
        // Por enquanto, suportamos 1 família por signature (suficiente para stdlib).
        // Para múltiplas famílias (ex: NUM NonZero), generalizar com produto cartesiano.
        if family_positions.len() == 1 {
            let (pos, family_name) = &family_positions[0];
            let instances = _struct_registry.all_instances(family_name);

            // Self-type desta signature (o primeiro param).
            let sig_self = sig.param_types.first().and_then(|ty| match ty {
                Ty::Struct(StructKey::Plain(name)) => Some(name.clone()),
                Ty::Prim(PrimTy::Int) => Some("Int".to_string()),
                Ty::Prim(PrimTy::Float) => Some("Float".to_string()),
                Ty::Prim(PrimTy::Rational) => Some("Rational".to_string()),
                _ => None,
            });

            for (concrete_alias, _) in &instances {
                // Filtro de signature fantasma: se o Self desta signature é
                // um tipo concreto (ex: "Int") e o concrete_alias da
                // instância é diferente (ex: "Complex"), isto significa que
                // estamos expandindo uma signature do prelude (ex: `/ ::
                // Int NonZero => Int`) para uma instância que pertence a um
                // tipo diferente. Neste caso, verificar se aquele tipo
                // (concrete_alias) define o método. Se não define, pular.
                //
                // Quando o Self é o mesmo concrete_alias (ex: `/ :: Int
                // NonZero => Int` expandindo para NonZero::Int), a
                // signature é legítima — o tipo base define o método.
                if let Some(ref self_name) = sig_self {
                    // A expansão cria uma signature para concrete_alias, mas
                    // o Self permanece self_name. Para que a signature
                    // expandida seja válida, o concrete_alias precisa ter
                    // uma overload do método. Caso contrário, é fantasma.
                    if self_name != concrete_alias {
                        let has_method = method_selfs
                            .get(&sig.name)
                            .map(|set| set.contains(&**concrete_alias))
                            .unwrap_or(false);
                        if !has_method {
                            continue;
                        }
                    }
                }

                let instance_key =
                    StructKey::Instance(family_name.clone(), concrete_alias.to_string());
                let mut new_params = sig.param_types.clone();
                new_params[*pos] = Ty::Struct(instance_key);

                let new_sig = Signature {
                    name: sig.name.clone(),
                    param_types: new_params.clone(),
                    return_type: sig.return_type.clone(),
                    ffi_symbol: sig.ffi_symbol.clone(),
                    is_associative: sig.is_associative,
                    associative_neutral: sig.associative_neutral,
                    is_action: sig.is_action,
                    is_commutative: sig.is_commutative,
                    type_params: sig.type_params.clone(),
                    param_names: sig.param_names.clone(),
                    param_defaults: sig.param_defaults.clone(),
                };
                new_sigs.push(new_sig);

                // NÃO expandir FunctionDefs com corpo Kata.
                // Funções FFI (sem corpo) são expandidas via Signature apenas.
                // Funções com corpo Kata mantêm Family nos param_types — o
                // dispatch resolve Family ↔ Instance no call site via
                // match_score (Family ↔ Instance = exact).
            }

            // Marcar a signature original para remoção.
            // (Fazemos isso adiando: não adicionamos a original de volta.)
        }
    }

    // Se houve expansão, substituir as signatures originais pelas expandidas.
    if !new_sigs.is_empty() {
        // Coletar nomes das signatures que foram expandidas (têm Family nos params).
        let expanded_names: Vec<(String, Vec<Ty>)> = signatures
            .iter()
            .filter(|sig| {
                sig.param_types
                    .iter()
                    .any(|ty| matches!(ty, Ty::Struct(StructKey::Family(_))))
            })
            .map(|sig| (sig.name.clone(), sig.param_types.clone()))
            .collect();

        // Remover signatures originais que foram expandidas.
        signatures.retain(|sig| {
            !expanded_names
                .iter()
                .any(|(name, params)| sig.name == *name && sig.param_types == *params)
        });

        // Adicionar signatures expandidas.
        signatures.extend(new_sigs);

        // FunctionDefs NÃO são removidos — mantêm Family nos param_types.
        // O dispatch encontra a Signature expandida (Instance concreta)
        // para FFI, e encontra a Signature original (Family) para corpos
        // Kata via match_score (Family ↔ Instance = exact).
    }
}
