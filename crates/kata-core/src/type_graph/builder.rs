//! Construção do `TypeGraph` a partir dos registries.
//!
//! [`TypeGraphBuilder`](self::TypeGraphBuilder) converte
//! `StructRegistry` + `EnumRegistry` + `InterfaceRegistry`
//! + `RefinesRegistry` em um [`TypeGraph`].

use crate::ty::PrimTy;
use crate::ty::Ty;

use super::{TypeEdge, TypeGraph, TypeKind};

/// Builder que converte `StructRegistry` + `EnumRegistry` + `InterfaceRegistry`
/// + `RefinesRegistry` em um `TypeGraph`.
///
/// Cada registry contribui com um aspecto da classificação:
/// - `StructRegistry` → Struct, Family, Refined, Alias
/// - `EnumRegistry` → GenericEnum, Enum
/// - `InterfaceRegistry` → Interface, Implements, Supertrait
/// - `RefinesRegistry` → Refines
pub struct TypeGraphBuilder<'a> {
    pub struct_reg: &'a crate::StructRegistry,
    pub enum_reg: &'a crate::EnumRegistry,
    pub iface_reg: &'a crate::InterfaceRegistry,
    pub refines_reg: &'a crate::RefinesRegistry,
}

impl<'a> TypeGraphBuilder<'a> {
    pub fn build(&self, origin: &str) -> TypeGraph {
        let mut graph = TypeGraph::new();

        // ── Primitivos do prelude ──
        // Sempre presentes, independentemente do origin.
        graph.insert("Int", TypeKind::Primitive(PrimTy::Int), "core");
        graph.insert("Float", TypeKind::Primitive(PrimTy::Float), "core");
        graph.insert("Text", TypeKind::Primitive(PrimTy::Text), "core");
        graph.insert("Rational", TypeKind::Primitive(PrimTy::Rational), "core");
        graph.insert("Unit", TypeKind::Intrinsic, "core");
        graph.insert("Byte", TypeKind::Intrinsic, "core");
        graph.insert("Bytes", TypeKind::Intrinsic, "core");
        graph.insert("Boolean", TypeKind::Enum, "core");

        // ── StructRegistry → Struct, Family, Refined, Alias ──
        self.populate_from_struct_registry(&mut graph, origin);

        // ── EnumRegistry → GenericEnum, Enum ──
        self.populate_from_enum_registry(&mut graph, origin);

        // ── InterfaceRegistry → Interface, Implements, Supertrait ──
        self.populate_from_interface_registry(&mut graph, origin);

        // ── RefinesRegistry → Refines ──
        self.populate_from_refines_registry(&mut graph, origin);

        graph
    }

    fn populate_from_struct_registry(&self, graph: &mut TypeGraph, origin: &str) {
        // Primeiro: registrar famílias (antes das instâncias, para que
        // as arestas Instance possam referenciar o nó Family).
        for family_name in self.struct_reg.all_family_names() {
            let instances: Vec<String> = self
                .struct_reg
                .all_instances(&family_name)
                .iter()
                .map(|(concrete, _)| concrete.to_string())
                .collect();
            graph.insert(
                &family_name,
                TypeKind::Family {
                    instances: instances.clone(),
                },
                origin,
            );
            for concrete in &instances {
                graph.add_edge(
                    &family_name,
                    TypeEdge::Instance {
                        concrete: concrete.clone(),
                    },
                    concrete,
                );
            }
        }

        // Segundo: iterar sobre todas as entradas para registrar
        // Structs, Refineds, Aliases. Pular instâncias de família
        // (is_instance_of.is_some()) — já cobertas acima.
        for (_entry_origin, key, info) in self.struct_reg.iter_all() {
            // Instâncias de família: já registradas como arestas.
            if info.is_instance_of.is_some() {
                continue;
            }

            let name = key.name();

            // Refined concreto: tem predicates e alias_of.
            if info.predicates.is_some() {
                let alias_target = info.alias_of.clone().unwrap_or_default();
                graph.insert(
                    name,
                    TypeKind::Refined {
                        alias_target: alias_target.clone(),
                        predicates: info.predicates.clone().unwrap_or_default(),
                    },
                    origin,
                );
                if !alias_target.is_empty() {
                    graph.add_edge(name, TypeEdge::Alias, &alias_target);
                }
                continue;
            }

            // Alias (newtype): tem alias_of sem predicates.
            if let Some(ref alias) = info.alias_of {
                graph.insert(
                    name,
                    TypeKind::Alias {
                        target: alias.clone(),
                    },
                    origin,
                );
                graph.add_edge(name, TypeEdge::Alias, alias);
                continue;
            }

            // Struct com campos.
            if !info.fields.is_empty() {
                graph.insert(name, TypeKind::Struct, origin);
                continue;
            }

            // Struct sem campos e sem alias — tipo opaco (ex: data Int () @ffi).
            // Não registra no grafo — primitivos já estão acima.
        }
    }

    fn populate_from_enum_registry(&self, graph: &mut TypeGraph, origin: &str) {
        for name in self.enum_reg.names() {
            if self.enum_reg.is_generic(name) {
                // Enum genérico: tem type params.
                let type_params: Vec<String> = self
                    .enum_reg
                    .type_params_of(name)
                    .map(|tps| tps.to_vec())
                    .unwrap_or_default();
                graph.insert(name, TypeKind::GenericEnum { type_params }, origin);
                // Adiciona arestas para cada type param.
                for tp in self.enum_reg.type_params_of(name).unwrap_or(&[]) {
                    graph.add_edge(
                        name,
                        TypeEdge::GenericParam {
                            name: tp.to_string(),
                        },
                        tp,
                    );
                }
            } else {
                // Enum monomórfico.
                graph.insert(name, TypeKind::Enum, origin);
            }
        }
    }

    fn populate_from_interface_registry(&self, graph: &mut TypeGraph, origin: &str) {
        // Interfaces.
        for iface in self.iface_reg.all_interfaces() {
            graph.insert(
                &iface.name,
                TypeKind::Interface {
                    supertraits: iface.supertraits.clone(),
                },
                origin,
            );
            for st in &iface.supertraits {
                graph.add_edge(&iface.name, TypeEdge::Supertrait { name: st.clone() }, st);
            }
        }

        // Implementations.
        for impl_entry in self.iface_reg.impls_view() {
            graph.add_edge(
                &impl_entry.type_name,
                TypeEdge::Implements {
                    interface: impl_entry.interface_name.clone(),
                },
                &impl_entry.interface_name,
            );
        }
    }

    fn populate_from_refines_registry(&self, graph: &mut TypeGraph, _origin: &str) {
        for entry in self.refines_reg.all_entries() {
            graph.add_edge(
                &entry.type_name,
                TypeEdge::Refines {
                    interface: entry.interface_name.clone(),
                },
                &entry.interface_name,
            );
        }
    }
}

/// Extrai o nome de primitivo de um `Ty` para comparação de instâncias.
pub(super) fn prim_name(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Prim(PrimTy::Int) => Some("Int".to_string()),
        Ty::Prim(PrimTy::Float) => Some("Float".to_string()),
        Ty::Prim(PrimTy::Rational) => Some("Rational".to_string()),
        Ty::Prim(PrimTy::Text) => Some("Text".to_string()),
        Ty::Struct(key) => Some(key.name().to_string()),
        _ => None,
    }
}
