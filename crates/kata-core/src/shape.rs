//! `TypeShape` — projeção runtime de `Ty` para reflexão estrutural.
//!
//! Descarta InferVar/Generic/Interface (mapeados para Unit graceful).
//! O codegen emite `register_type(ptr, type_id)` após cada `alloc_arc`,
//! permitindo que `typeof` e `pretty_print` funcionem em runtime.
//!
//! Scaffolding para fios futuros (reflexão runtime, debugger).
#![allow(dead_code)]
//!
//! `type_id: u32` — identificador atribuído em compile-time para cada
//! `Ty` distinto no módulo. Serve de chave para a type table do runtime.

use crate::ty::Ty;

/// Projeção runtime de `Ty`.
///
/// `TypeShape` tem `Box`, `String`, `Vec` — sem layout C-ABI estável.
/// Por isso a type table é registrada Rust-to-Rust pelo driver, não
/// serializada através da fronteira C-ABI (manual §maquinaria-interna).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TypeShape {
    /// Tipos primitivos (Int, Float, Text, Rational).
    Prim,
    /// Unit (zero-sized).
    Unit,
    /// Struct com campos nomeados.
    Struct {
        name: String,
        fields: Vec<(String, TypeShape)>,
    },
    /// Sum (enum) com variantes.
    Sum {
        name: String,
        variants: Vec<(String, Option<Box<TypeShape>>)>,
    },
    /// Função.
    Func {
        params: Vec<TypeShape>,
        ret: Box<TypeShape>,
    },
    /// Tupla heterogênea (antecipado de Fio 5; Fio 2 usa para patterns).
    Tuple { elements: Vec<TypeShape> },
}

impl TypeShape {
    /// Verifica se o tipo é heap-allocated (ponteiro no runtime).
    ///
    /// Text, Rational, Struct, Sum, Tuple são heap types.
    /// Int, Float, Unit não são.
    pub(crate) fn is_heap_type(&self) -> bool {
        match self {
            TypeShape::Prim => false, // Int e Float são i64/f64 inline
            TypeShape::Unit => false,
            TypeShape::Struct { .. } => true,
            TypeShape::Sum { .. } => true,
            TypeShape::Func { .. } => true,
            TypeShape::Tuple { .. } => true,
        }
    }
}

impl Ty {
    /// Projeta `Ty` para `TypeShape` (descarta InferVar → Unit).
    pub(crate) fn to_shape(&self) -> TypeShape {
        match self {
            Ty::Prim(_) => TypeShape::Prim,
            Ty::Unit => TypeShape::Unit,
            Ty::Struct(name) => TypeShape::Struct {
                name: name.clone(),
                fields: Vec::new(),
            },
            Ty::Sum(name) => TypeShape::Sum {
                name: name.clone(),
                variants: Vec::new(),
            },
            Ty::Function(params, ret) => TypeShape::Func {
                params: params.iter().map(|t| t.to_shape()).collect(),
                ret: Box::new(ret.to_shape()),
            },
            Ty::Tuple(elements) => TypeShape::Tuple {
                elements: elements.iter().map(|t| t.to_shape()).collect(),
            },
            // Tolerante a tipos não-resolvidos durante typeck
            Ty::InferVar(_) => TypeShape::Unit,
            // Var: variável de tipo — não tem shape runtime próprio.
            Ty::Var(_) => TypeShape::Unit,
            // Generic: projeta como Sum (é um enum instanciado).
            Ty::Generic(name, _) => TypeShape::Sum {
                name: name.clone(),
                variants: Vec::new(),
            },
            // Interface: não é um tipo concreto — mapeia para Unit graceful.
            Ty::Interface(_) => TypeShape::Unit,
            // List/Array/Range: coleções intrínsecas — heap types (ponteiro na arena).
            // Mapeadas como Struct com nome para reflexão runtime (typeof, pretty_print).
            Ty::List(_) => TypeShape::Struct {
                name: "List".into(),
                fields: Vec::new(),
            },
            Ty::Array(_) => TypeShape::Struct {
                name: "Array".into(),
                fields: Vec::new(),
            },
            Ty::Range(_) => TypeShape::Struct {
                name: "Range".into(),
                fields: Vec::new(),
            },
        }
    }
}

/// Identificador de tipo — u32 atribuído em compile-time para cada
/// `Ty` distinto no módulo.
pub(crate) type TypeId = u32;
