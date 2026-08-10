//! Cache key utilities para `@cache`.
//!
//! Extraído de `function_def.rs` — agrupa:
//! - `canonical_fn_id` (hash FNV-1a de nome + tipos + body canônico)
//! - `canonical_expr` (serialização estável de TypedExpr sem spans/ty)
//! - `build_type_descriptor` / `write_descriptor` (byte array C-ABI estável
//!   descrevendo o layout do tipo para o runtime serializar o valor por
//!   conteúdo)
//!
//! Todas essas funções só são usadas pelo caminho `@cache` dentro de
//! `function_def.rs` — extraí-las aqui separa o "machinery de cache key"
//! do "machinery de lowering de função".

use kata_core::ty::{PrimTy, Ty};
use kata_inference::{TypedExpr, TypedExprKind, TypedLambdaClause};

/// Computa um fn_id canônico a partir de nome + param_types + body.
///
/// O fn_id diferencia:
/// - Overloads monomórficos (mesmo nome, tipos diferentes)
/// - Bodies diferentes com mesma assinatura (REPL iter — body muda, fn_id muda)
/// - Funções diferentes com mesmo nome em programas diferentes
///
/// Serializa nome + tipos + body em uma string canônica (sem spans, sem ty)
/// e aplica FNV-1a para produzir um i64 estável.
pub(crate) fn canonical_fn_id(
    name: &str,
    param_types: &[Ty],
    clauses: &[TypedLambdaClause],
) -> i64 {
    let mut buf = String::new();
    buf.push_str(name);
    buf.push('|');
    for ty in param_types {
        buf.push_str(&format!("{ty}"));
        buf.push(',');
    }
    buf.push('|');
    for clause in clauses {
        // Serializa patterns da cláusula
        for pat in &clause.patterns {
            buf.push_str(&format!("{:?}", pat.node));
            buf.push(',');
        }
        buf.push('|');
        // Serializa body canônico (sem spans)
        canonical_expr(&clause.body.node, &mut buf);
        buf.push(';');
    }
    // FNV-1a hash (usando u64, cast para i64 no final)
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in buf.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}

/// Serializa um TypedExpr em string canônica (sem spans, sem ty).
fn canonical_expr(expr: &TypedExpr, buf: &mut String) {
    match &expr.kind {
        TypedExprKind::IntLit { text } => {
            buf.push_str("Int:");
            buf.push_str(text);
        }
        TypedExprKind::FloatLit { text } => {
            buf.push_str("Float:");
            buf.push_str(text);
        }
        TypedExprKind::TextLit { text } => {
            buf.push_str("Text:");
            buf.push_str(text);
        }
        TypedExprKind::Unit => buf.push_str("Unit"),
        TypedExprKind::Ident { name } => {
            buf.push_str("Ident:");
            buf.push_str(name);
        }
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            buf.push_str("Call(");
            canonical_expr(&callee.node, buf);
            buf.push(',');
            for arg in args {
                canonical_expr(&arg.node, buf);
                buf.push(',');
            }
            buf.push(')');
            if let Some(ffi) = ffi_symbol {
                buf.push_str(":ffi:");
                buf.push_str(ffi);
            }
        }
        TypedExprKind::TypeAscription { expr, .. } => {
            buf.push_str("Ascribe(");
            canonical_expr(&expr.node, buf);
            buf.push(')');
        }
        TypedExprKind::Grouping { inner } => {
            buf.push_str("Group(");
            canonical_expr(&inner.node, buf);
            buf.push(')');
        }
        TypedExprKind::Tuple { elements } => {
            buf.push_str("Tuple(");
            for el in elements {
                canonical_expr(&el.node, buf);
                buf.push(',');
            }
            buf.push(')');
        }
        TypedExprKind::StructConstruct {
            struct_name,
            values,
        } => {
            buf.push_str("Struct(");
            buf.push_str(struct_name);
            buf.push(',');
            for v in values {
                canonical_expr(&v.node, buf);
                buf.push(',');
            }
            buf.push(')');
        }
        TypedExprKind::FieldAccess {
            expr,
            struct_name,
            field_name,
            ..
        } => {
            buf.push_str("Field(");
            canonical_expr(&expr.node, buf);
            buf.push(',');
            buf.push_str(struct_name);
            buf.push(',');
            buf.push_str(field_name);
            buf.push(')');
        }
        TypedExprKind::IndexAccess { expr, index, .. } => {
            buf.push_str("Index(");
            canonical_expr(&expr.node, buf);
            buf.push(',');
            buf.push_str(&index.to_string());
            buf.push(')');
        }
        TypedExprKind::Let { name, value } => {
            buf.push_str("Let(");
            buf.push_str(name);
            buf.push(',');
            canonical_expr(&value.node, buf);
            buf.push(')');
        }
        TypedExprKind::LetDestruct {
            temp_name,
            value,
            bindings,
        } => {
            buf.push_str("LetDestruct(");
            buf.push_str(temp_name);
            buf.push(',');
            canonical_expr(&value.node, buf);
            buf.push(',');
            for (n, e) in bindings {
                buf.push_str(n);
                buf.push('=');
                canonical_expr(&e.node, buf);
                buf.push(',');
            }
            buf.push(')');
        }
        TypedExprKind::VariantQual {
            enum_name, variant, ..
        } => {
            buf.push_str("Variant(");
            buf.push_str(enum_name);
            buf.push(',');
            buf.push_str(variant);
            buf.push(')');
        }
        TypedExprKind::VariantConstruct {
            enum_name,
            variant,
            payload,
            ..
        } => {
            buf.push_str("VariantC(");
            buf.push_str(enum_name);
            buf.push(',');
            buf.push_str(variant);
            buf.push(',');
            canonical_expr(&payload.node, buf);
            buf.push(')');
        }
        TypedExprKind::Lambda {
            func_name,
            param_types,
            ..
        } => {
            buf.push_str("Lambda(");
            if let Some(n) = func_name {
                buf.push_str(n);
            }
            buf.push(',');
            for ty in param_types {
                buf.push_str(&format!("{ty}"));
                buf.push(',');
            }
            buf.push(')');
        }
        TypedExprKind::Return(expr) => {
            buf.push_str("Return(");
            canonical_expr(&expr.node, buf);
            buf.push(')');
        }
        TypedExprKind::Fork {
            action_name,
            action_expr,
            ..
        } => {
            buf.push_str("Fork(");
            buf.push_str(action_name);
            buf.push(',');
            canonical_expr(&action_expr.node, buf);
            buf.push(')');
        }
        TypedExprKind::ActionCall {
            callee,
            ffi_symbol,
            args,
            ..
        } => {
            buf.push_str("ActionCall(");
            buf.push_str(callee);
            if let Some(ffi) = ffi_symbol {
                buf.push_str(":ffi:");
                buf.push_str(ffi);
            }
            buf.push(',');
            canonical_expr(&args.node, buf);
            buf.push(')');
        }
        TypedExprKind::Match { scrutinee, arms } => {
            buf.push_str("Match(");
            canonical_expr(&scrutinee.node, buf);
            buf.push(',');
            for arm in arms {
                if let Some(p) = &arm.pattern {
                    buf.push_str(&format!("{:?}", p.node));
                } else {
                    buf.push_str("otherwise");
                }
                buf.push('=');
                canonical_expr(&arm.body.node, buf);
                buf.push(',');
            }
            buf.push(')');
        }
        TypedExprKind::Comptime { expr } => {
            buf.push_str("Comptime(");
            canonical_expr(&expr.node, buf);
            buf.push(')');
        }
        TypedExprKind::Block { stmts } => {
            buf.push_str("Block(");
            for stmt in stmts {
                canonical_expr(&stmt.node, buf);
                buf.push(',');
            }
            buf.push(')');
        }
        // Catch-all para variants que não afetam o fn_id de funções @cache
        // (Int => Int): coleções, CSP, loops, type introspection, etc.
        // Estes nós não aparecem em funções puras Int => Int.
        _ => buf.push_str("Other"),
    }
}

// ── Type descriptor para cache key serialização ──────────────────────
//
// Construído em compile-time pelo codegen. Byte array C-ABI estável que
// descreve o layout do tipo para o runtime serializar o valor por conteúdo.
//
// Tags:
//   0x00 Unit     — 0 bytes
//   0x01 Int      — 8 bytes valor imediato
//   0x02 Float    — 8 bytes valor (f64 bits)
//   0x03 Text     — C string: len (4 bytes LE) + bytes
//   0x04 List(T)  — percorre cons cells, serializa cada head com T
//   0x05 Struct   — n_fields (u8) + field descriptors
//   0x06 Tuple    — n_elems (u8) + elem descriptors
//   0x07 Sum      — tag (8 bytes) + payload (8 bytes crus)
//                  Limitação: payload não serializado recursivamente sem
//                  enum_registry. Dois Sums com mesmo payload em endereços
//                  diferentes terão keys diferentes.

const TD_UNIT: u8 = 0x00;
const TD_INT: u8 = 0x01;
const TD_FLOAT: u8 = 0x02;
const TD_TEXT: u8 = 0x03;
const TD_LIST: u8 = 0x04;
const TD_STRUCT: u8 = 0x05;
const TD_TUPLE: u8 = 0x06;
const TD_SUM: u8 = 0x07;
const TD_BYTE: u8 = 0x08;
const TD_BYTES: u8 = 0x09;

/// Constrói um type descriptor (byte array) para um `Ty`.
pub(super) fn build_type_descriptor(
    ty: &Ty,
    struct_registry: &kata_core::StructRegistry,
) -> Vec<u8> {
    let mut buf = Vec::new();
    write_descriptor(&mut buf, ty, struct_registry);
    buf
}

fn write_descriptor(buf: &mut Vec<u8>, ty: &Ty, struct_registry: &kata_core::StructRegistry) {
    match ty {
        Ty::Unit => buf.push(TD_UNIT),
        Ty::Prim(PrimTy::Int) => buf.push(TD_INT),
        Ty::Prim(PrimTy::Float) => buf.push(TD_FLOAT),
        Ty::Prim(PrimTy::Text) => buf.push(TD_TEXT),
        Ty::Prim(_) => buf.push(TD_INT),
        Ty::List(elem) => {
            buf.push(TD_LIST);
            write_descriptor(buf, elem, struct_registry);
        }
        Ty::Tuple(elements) => {
            buf.push(TD_TUPLE);
            buf.push(elements.len() as u8);
            for elem in elements {
                write_descriptor(buf, elem, struct_registry);
            }
        }
        Ty::Struct(name) => {
            buf.push(TD_STRUCT);
            if let Some(info) = struct_registry.get(name) {
                buf.push(info.fields.len() as u8);
                for field in &info.fields {
                    write_descriptor(buf, &field.ty, struct_registry);
                }
            } else {
                buf.push(1);
                buf.push(TD_INT);
            }
        }
        Ty::Sum(_) | Ty::Generic(_, _) => {
            buf.push(TD_SUM);
        }
        Ty::Byte => {
            buf.push(TD_BYTE);
        }
        Ty::Bytes => {
            buf.push(TD_BYTES);
        }
        _ => {
            buf.push(TD_INT);
        }
    }
}
