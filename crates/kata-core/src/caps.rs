//! Capacidades e representação de tipos — `TypeCaps`, `Repr`, `CapsIndex`.
//!
//! Derivado das registries dinâmicas (`TypeGraph`, `StructRegistry`,
//! `InterfaceRegistry`, `InlineFnTable`) que o compilador já constrói.
//! Permite que o motor Maranget consulte capacidade (ORD? EQ? NUM?) e
//! representação de dados sem hard-codar nomes de tipo. Tipos de usuário
//! funcionam automaticamente.
//!
//! ## Eixo capacidade vs transparência
//!
//! - **Capacidade** = o que a linguagem permite (implementa ORD? EQ? NUM?)
//!   — via `TypeGraph::type_implements`.
//! - **Transparência** = o que o compilador consegue provar (const-eval?
//!   inline?) — via `InlineFnTable` (não acessível aqui; transparência
//!   fica em `InferCtx` e é consultada separadamente).
//! - Capacidade sem transparência → fallback estrutural conservador
//!   (Missing/otherwise), nunca erro falso.

use crate::struct_registry::StructRegistry;
use crate::ty::{PrimTy, Ty};
use crate::type_graph::TypeGraph;

/// Representação de dados de um tipo — como o valor é estruturado
/// fisicamente. Ortogonal a capacidade (ord/eq).
///
/// Determina como `Constructor::Literal` é serializado e como
/// `enum_refined_domain` enumera valores.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Repr {
    /// Inteiro discreto de 64 bits. Domínio enumerável via intervalo.
    Int,
    /// Ponto flutuante. Domínio NÃO enumerável (densidade infinita).
    Float,
    /// Racional (par num/den). Domínio NÃO enumerável diretamente.
    Rational,
    /// Texto. Domínio NÃO enumerável.
    Text,
    /// Unit — único valor `()`.
    Unit,
    /// Boolean — enum com duas variantes (não é Repr, é Sum).
    /// Mas para fins de const-eval, Boolean é enumerável.
    Bool,
    /// Struct com N campos — cada campo tem sua própria Repr.
    /// Domínio potencialmente enumerável se todos os campos têm Repr
    /// discreta e o tipo implementa EQ + const-eval.
    Struct(Vec<Repr>),
    /// Representação desconhecida/não-suportada — fallback opaco.
    /// O motor trata como tipo infinito sem construtores nomeados.
    Opaque,
}

/// Capacidades de um tipo — o que a linguagem permite fazer com ele.
///
/// Derivado de `TypeGraph::type_implements` (que consulta as
/// declarações `implements` do usuário + stdlib). Ortogonal a `Repr`:
/// `Repr` descreve a representação; `TypeCaps` descreve quais operações
/// são válidas sobre essa representação.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCaps {
    /// Implementa `ORD` (ordenabilidade: `<`, `<=`, `>`, `>=`).
    /// `ORD implements EQ`, então `ord` implica `eq`.
    pub ord: bool,
    /// Implementa `EQ` (igualdade: `=`, `!=`).
    pub eq: bool,
    /// Implementa `NUM` (aritmética: `+`, `-`, `*`, `/`).
    pub num: bool,
    /// Representação de dados.
    pub repr: Repr,
}

impl TypeCaps {
    /// Caps para um tipo que não existe no grafo — fallback conservador.
    /// Sem capacidades, repr opaca.
    fn unknown() -> Self {
        TypeCaps {
            ord: false,
            eq: false,
            num: false,
            repr: Repr::Opaque,
        }
    }
}

/// Índice de capacidades construído a partir de `TypeGraph` e
/// `StructRegistry`. Consulta por nome de tipo.
///
/// Construído uma vez no início de `infer_module` a partir de `InferCtx`.
/// `MarangetEnv` recebe `&CapsIndex` — preserva a separação motor
/// puro/typeck.
#[derive(Debug, Clone)]
pub struct CapsIndex {
    /// Cache de caps por nome de tipo (após `follow_alias`).
    /// Key = nome do tipo após resolução de alias.
    caps: std::collections::HashMap<String, TypeCaps>,
}

/// Valor constante canônico — resultado de const-eval de uma expressão.
///
/// Corresponde a um `Repr`: `Repr::Int` → `ConstVal::Int(i64)`,
/// `Repr::Float` → `ConstVal::Float(f64)`, etc.
/// `Repr::Struct` → `ConstVal::Struct(Vec<ConstVal>)` (campos em ordem).
#[derive(Debug, Clone, PartialEq)]
pub enum ConstVal {
    /// Inteiro de 64 bits.
    Int(i64),
    /// Ponto flutuante.
    Float(f64),
    /// Racional como par (numerador, denominador).
    Rat(i64, i64),
    /// Boolean.
    Bool(bool),
    /// Texto.
    Text(String),
    /// Unit.
    Unit,
    /// Struct com campos — cada campo é um `ConstVal`.
    Struct(Vec<ConstVal>),
}

impl ConstVal {
    /// Compara dois `ConstVal` para ordenação (usado por `extract_bound`
    /// e `enum_refined_domain` quando o tipo implementa ORD).
    ///
    /// Float e Rat comparam por valor. Struct compara lexicograficamente.
    /// Retorna `None` se os tipos são incomparáveis.
    pub fn compare(&self, other: &ConstVal) -> Option<std::cmp::Ordering> {
        use std::cmp::Ordering;
        match (self, other) {
            (ConstVal::Int(a), ConstVal::Int(b)) => Some(a.cmp(b)),
            (ConstVal::Float(a), ConstVal::Float(b)) => a.partial_cmp(b),
            (ConstVal::Rat(a_num, a_den), ConstVal::Rat(b_num, b_den)) => {
                // Cross-multiplication: a_num/a_den < b_num/b_den
                // sse a_num * b_den < b_num * a_den (assumindo dens > 0).
                if *a_den <= 0 || *b_den <= 0 {
                    return None;
                }
                let left = (*a_num as i128) * (*b_den as i128);
                let right = (*b_num as i128) * (*a_den as i128);
                Some(left.cmp(&right))
            }
            (ConstVal::Bool(a), ConstVal::Bool(b)) => Some(a.cmp(b)),
            (ConstVal::Text(a), ConstVal::Text(b)) => Some(a.cmp(b)),
            (ConstVal::Unit, ConstVal::Unit) => Some(Ordering::Equal),
            (ConstVal::Struct(a), ConstVal::Struct(b)) => {
                for (av, bv) in a.iter().zip(b.iter()) {
                    match av.compare(bv)? {
                        Ordering::Equal => continue,
                        ord => return Some(ord),
                    }
                }
                Some(a.len().cmp(&b.len()))
            }
            _ => None, // tipos diferentes
        }
    }

    /// Serializa para string no formato de `Constructor::Literal`.
    /// Round-trip com `pattern_ctor` → `literal_to_string`.
    pub fn to_ctor_string(&self, repr: &Repr) -> String {
        match (repr, self) {
            (Repr::Int, ConstVal::Int(v)) => format!("Int:{}", v),
            (Repr::Float, ConstVal::Float(v)) => format!("Float:{}", v),
            (Repr::Rational, ConstVal::Rat(n, d)) => format!("Rat:{}|{}", n, d),
            (Repr::Text, ConstVal::Text(s)) => format!("Text:{}", s),
            (Repr::Bool, ConstVal::Bool(b)) => format!("Bool:{}", b),
            (Repr::Unit, ConstVal::Unit) => "Unit:()".to_string(),
            (Repr::Struct(_), ConstVal::Struct(fields)) => {
                let inner: Vec<String> = fields
                    .iter()
                    .map(|v| match v {
                        ConstVal::Int(v) => v.to_string(),
                        ConstVal::Float(v) => v.to_string(),
                        ConstVal::Rat(n, d) => format!("{}|{}", n, d),
                        ConstVal::Bool(b) => b.to_string(),
                        ConstVal::Text(s) => s.clone(),
                        ConstVal::Unit => "()".to_string(),
                        ConstVal::Struct(_) => format!("{:?}", v),
                    })
                    .collect();
                format!("Struct:{}", inner.join("|"))
            }
            _ => format!("Other:{:?}", self),
        }
    }
}

impl CapsIndex {
    /// Constrói o índice a partir de `TypeGraph` e `StructRegistry`.
    ///
    /// Percorre todos os nós do `TypeGraph` e computa `TypeCaps` para cada
    /// tipo conhecido. Tipos não registrados ficam de fora do cache e
    /// retornam `TypeCaps::unknown()` na consulta.
    pub fn new(type_graph: &TypeGraph, struct_registry: &StructRegistry) -> Self {
        let mut caps = std::collections::HashMap::new();

        // Tipos primitivos do prelude — sempre presentes.
        for (name, prim) in [
            ("Int", PrimTy::Int),
            ("Float", PrimTy::Float),
            ("Text", PrimTy::Text),
            ("Rational", PrimTy::Rational),
        ] {
            let c = compute_caps(type_graph, name, Some(prim), struct_registry);
            caps.insert(name.to_string(), c);
        }

        // Boolean é enum, não primitivo — mas tem repr Bool para const-eval.
        let bool_caps = compute_caps(type_graph, "Boolean", None, struct_registry);
        caps.insert("Boolean".to_string(), bool_caps);

        // Pré-cacheia todos os tipos de usuário do StructRegistry.
        // Isto garante que refineds/aliases como `UmOuDois` tenham suas
        // caps computadas com resolução de alias via `follow_alias`.
        for (_origin, key, _info) in struct_registry.iter_all() {
            let name = key.name();
            if !caps.contains_key(name) {
                let c = compute_caps(type_graph, name, None, struct_registry);
                caps.insert(name.to_string(), c);
            }
        }

        CapsIndex { caps }
    }

    /// Consulta as capacidades de um tipo pelo nome (após `follow_alias`).
    ///
    /// Se o tipo já está no cache, retorna do cache. Caso contrário,
    /// computa on-demand e armazena.
    pub fn get(&self, type_name: &str) -> TypeCaps {
        self.caps
            .get(type_name)
            .cloned()
            .unwrap_or_else(TypeCaps::unknown)
    }

    /// Consulta as capacidades de um `Ty`.
    ///
    /// Resolve `Ty::Struct(key)` via `follow_alias` do `TypeGraph`,
    /// `Ty::Sum`/`Ty::Generic` como enum (sem repr discreta),
    /// `Ty::Prim` direto.
    pub fn get_for_ty(&self, ty: &Ty) -> TypeCaps {
        match ty {
            Ty::Prim(PrimTy::Int) => self.get("Int"),
            Ty::Prim(PrimTy::Float) => self.get("Float"),
            Ty::Prim(PrimTy::Text) => self.get("Text"),
            Ty::Prim(PrimTy::Rational) => self.get("Rational"),
            Ty::Unit => TypeCaps {
                ord: false,
                eq: true,
                num: false,
                repr: Repr::Unit,
            },
            Ty::Sum(name) | Ty::Generic(name, _) => {
                // Enum — sem repr discreta (variantes, não literais).
                // Capacidades via TypeGraph.
                let c = self.get(name);
                if c.repr == Repr::Opaque {
                    // Enum desconhecido — mantém Opaque mas pode ter eq
                    // se implementar EQ.
                    TypeCaps {
                        ord: self.get(name).ord,
                        eq: self.get(name).eq,
                        num: self.get(name).num,
                        repr: Repr::Opaque,
                    }
                } else {
                    c
                }
            }
            Ty::Struct(key) => {
                // Refined/alias/struct — resolve pelo nome público.
                self.get(key.name())
            }
            _ => TypeCaps::unknown(),
        }
    }
}

/// Computa `TypeCaps` para um tipo pelo nome, consultando `TypeGraph`
/// para capacidades (ORD, EQ, NUM) e derivando `Repr` do `PrimTy` ou
/// `StructRegistry`.
fn compute_caps(
    type_graph: &TypeGraph,
    name: &str,
    prim: Option<PrimTy>,
    struct_registry: &StructRegistry,
) -> TypeCaps {
    // Capacidades via TypeGraph. Para refineds/aliases, herda capacidades
    // do tipo base (follow_alias resolve a cadeia).
    let base = type_graph.follow_alias(name);
    let caps_name = if base != name { &base } else { name };
    let ord = type_graph.type_implements(caps_name, "ORD");
    let eq = type_graph.type_implements(caps_name, "EQ") || ord; // ORD implica EQ
    let num = type_graph.type_implements(caps_name, "NUM");

    // Representação: PrimTy se primitivo, Bool se Boolean, Struct se tem
    // campos, senão resolve alias via follow_alias para encontrar o tipo
    // base e sua Repr.
    let repr = if let Some(p) = prim {
        match p {
            PrimTy::Int => Repr::Int,
            PrimTy::Float => Repr::Float,
            PrimTy::Text => Repr::Text,
            PrimTy::Rational => Repr::Rational,
        }
    } else if name == "Boolean" {
        Repr::Bool
    } else {
        // Tenta buscar campos no StructRegistry.
        let struct_info = struct_registry.get(name);
        if let Some(info) = struct_info
            && !info.fields.is_empty()
        {
            let field_reprs = info
                .fields
                .iter()
                .map(|f| repr_of_ty(&f.ty, type_graph, struct_registry))
                .collect();
            Repr::Struct(field_reprs)
        } else {
            // Sem campos — pode ser refined/alias. Resolve alias via
            // follow_alias para encontrar o tipo base e sua Repr.
            let base = type_graph.follow_alias(name);
            if base != name {
                // É alias/refined — computa repr do tipo base.
                let base_info = struct_registry.get(&base);
                if let Some(info) = base_info
                    && !info.fields.is_empty()
                {
                    let field_reprs = info
                        .fields
                        .iter()
                        .map(|f| repr_of_ty(&f.ty, type_graph, struct_registry))
                        .collect();
                    Repr::Struct(field_reprs)
                } else {
                    // Base é primitivo — mapeia por nome.
                    prim_repr_by_name(&base)
                }
            } else {
                Repr::Opaque
            }
        }
    };

    TypeCaps { ord, eq, num, repr }
}

/// Deriva `Repr` de um `Ty` recursivamente, consultando registries.
fn repr_of_ty(ty: &Ty, type_graph: &TypeGraph, struct_registry: &StructRegistry) -> Repr {
    match ty {
        Ty::Prim(PrimTy::Int) => Repr::Int,
        Ty::Prim(PrimTy::Float) => Repr::Float,
        Ty::Prim(PrimTy::Text) => Repr::Text,
        Ty::Prim(PrimTy::Rational) => Repr::Rational,
        Ty::Unit => Repr::Unit,
        Ty::Sum(name) | Ty::Generic(name, _) if name == "Boolean" => Repr::Bool,
        Ty::Struct(key) => {
            let base = type_graph.follow_alias(key.name());
            if base != key.name() {
                // É alias/refined — computa repr do tipo base.
                let struct_info = struct_registry.get(&base);
                if let Some(info) = struct_info
                    && !info.fields.is_empty()
                {
                    let field_reprs = info
                        .fields
                        .iter()
                        .map(|f| repr_of_ty(&f.ty, type_graph, struct_registry))
                        .collect();
                    Repr::Struct(field_reprs)
                } else {
                    // Base é primitivo — mapeia por nome.
                    prim_repr_by_name(&base)
                }
            } else {
                Repr::Opaque
            }
        }
        _ => Repr::Opaque,
    }
}

/// Mapeia nome de tipo primitivo para `Repr`.
fn prim_repr_by_name(name: &str) -> Repr {
    match name {
        "Int" => Repr::Int,
        "Float" => Repr::Float,
        "Text" => Repr::Text,
        "Rational" => Repr::Rational,
        _ => Repr::Opaque,
    }
}
