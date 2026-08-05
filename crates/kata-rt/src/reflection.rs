//! Reflexão de funções — sidecar table com metadata estática.
//!
//! O codegen emite um data symbol `__kata_fn_meta_table` com N entries
//! de 56 bytes cada. O runtime registra esta tabela no prólogo do entry
//! point (antes da execução do código do usuário) e consulta via binary
//! search quando `f.name` (ou outro field) é acessado em contexto dinâmico.
//!
//! Layout binário (56 bytes por entry):
//!   offset 0:  fn_ptr         (i64 — relocation resolvida pelo JIT)
//!   offset 8:  name_ptr        (i64 — ponteiro para string estática)
//!   offset 16: arity           (i64 — número de parâmetros)
//!   offset 24: param_types_ptr (i64 — ponteiro para array de string ptrs)
//!   offset 32: param_types_len (i64 — número de param types)
//!   offset 40: return_type_ptr (i64 — ponteiro para string estática)
//!   offset 48: is_action       (i64 — 0 = Function, 1 = Action)

use std::cell::RefCell;

/// Field IDs para `kata_rt_fn_meta_lookup`.
pub const FIELD_NAME: i64 = 0;
pub const FIELD_ARITY: i64 = 1;
pub const FIELD_PARAM_TYPES: i64 = 2;
pub const FIELD_RETURN_TYPE: i64 = 3;
pub const FIELD_IS_ACTION: i64 = 4;

/// Entry na sidecar table — lida do binário via ponteiro raw.
#[repr(C)]
#[derive(Clone, Copy)]
struct FnMetaEntry {
    fn_ptr: i64,
    name_ptr: i64,
    arity: i64,
    param_types_ptr: i64,
    param_types_len: i64,
    return_type_ptr: i64,
    is_action: i64,
}

const ENTRY_SIZE: usize = 56; // 7 * 8 bytes

/// Tabela de metadata de funções, ordenada por fn_ptr para binary search.
struct FnMetaTable {
    entries: Vec<FnMetaEntry>,
}

impl FnMetaTable {
    fn empty() -> Self {
        FnMetaTable {
            entries: Vec::new(),
        }
    }

    /// Binary search por fn_ptr. Retorna o índice ou None.
    fn lookup(&self, fn_ptr: i64) -> Option<&FnMetaEntry> {
        let mut lo = 0usize;
        let mut hi = self.entries.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let entry = &self.entries[mid];
            match entry.fn_ptr.cmp(&fn_ptr) {
                std::cmp::Ordering::Equal => return Some(entry),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }
}

thread_local! {
    static FN_META_TABLE: RefCell<FnMetaTable> = RefCell::new(FnMetaTable::empty());
}

/// Registra a sidecar table no TLS.
///
/// Lê `count` entries a partir de `ptr` (cada entry: 56 bytes), copia para
/// um Vec, ordena por fn_ptr (os fn_ptrs só são conhecidos após JIT finalize),
/// e armazena em TLS.
///
/// Chamada pelo codegen no prólogo do entry point, antes da execução do
/// código do usuário.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_register_fn_meta_table(ptr: i64, count: i64) {
    if ptr == 0 || count <= 0 {
        return;
    }
    let base = ptr as *const u8;
    let n = count as usize;
    let mut entries = Vec::with_capacity(n);
    for i in 0..n {
        // Entries começam após o header (count: i64 = 8 bytes).
        let offset = 8 + i * ENTRY_SIZE;
        let entry_ptr = unsafe { base.add(offset) as *const FnMetaEntry };
        let entry = unsafe { entry_ptr.read_unaligned() };
        entries.push(entry);
    }
    // Ordena por fn_ptr — necessário para binary search.
    // Os fn_ptrs só são conhecidos após finalize_definitions() no JIT.
    entries.sort_by_key(|e| e.fn_ptr);
    FN_META_TABLE.with(|t| {
        t.borrow_mut().entries = entries;
    });
}

/// Consulta um field da metadata de uma função via binary search.
///
/// `fn_ptr`: ponteiro da função (valor i64 em runtime).
/// `field`: 0=name, 1=arity, 2=param_types, 3=return_type, 4=is_action.
///
/// Retorna o valor do field como i64:
/// - name/return_type: ponteiro para string C (Text ptr)
/// - arity: inteiro SMI-tagged
/// - param_types: ponteiro para List (Cons cell de Text ptrs)
/// - is_action: 0 ou 1 (Boolean)
///
/// Retorna 0 se fn_ptr não encontrado (sentinel → string vazia, arity 0, etc).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fn_meta_lookup(fn_ptr: i64, field: i64) -> i64 {
    FN_META_TABLE.with(|t| {
        let table = t.borrow();
        match table.lookup(fn_ptr) {
            None => 0,
            Some(entry) => match field {
                FIELD_NAME => entry.name_ptr,
                FIELD_ARITY => crate::bigint::kata_rt_tag_int(entry.arity),
                FIELD_PARAM_TYPES => {
                    // Constrói List Cons na root arena a partir do array de
                    // string ptrs em param_types_ptr (com param_types_len elementos).
                    // Cada elemento é um Text ptr (C string). Constrói da cauda
                    // para a cabeça (fold right): Nil=0, depois cons(ptr[n-1], 0),
                    // cons(ptr[n-2], ...), etc.
                    let arena = crate::arena::kata_rt_get_root_arena_handle();
                    let n = entry.param_types_len as usize;
                    let base = entry.param_types_ptr as *const i64;
                    let mut list = 0i64; // Nil
                    for i in (0..n).rev() {
                        let text_ptr = unsafe { base.add(i).read_unaligned() };
                        list = crate::list::kata_rt_list_cons(text_ptr, list, arena);
                    }
                    list
                }
                FIELD_RETURN_TYPE => entry.return_type_ptr,
                FIELD_IS_ACTION => entry.is_action,
                _ => 0,
            },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(fn_ptr: i64, name: i64, arity: i64) -> FnMetaEntry {
        FnMetaEntry {
            fn_ptr,
            name_ptr: name,
            arity,
            param_types_ptr: 0,
            param_types_len: 0,
            return_type_ptr: 0,
            is_action: 0,
        }
    }

    #[test]
    fn binary_search_finds_entry() {
        let table = FnMetaTable {
            entries: vec![
                make_entry(100, 1, 2),
                make_entry(200, 2, 3),
                make_entry(300, 3, 1),
            ],
        };
        assert_eq!(table.lookup(200).unwrap().name_ptr, 2);
        assert_eq!(table.lookup(100).unwrap().arity, 2);
        assert_eq!(table.lookup(300).unwrap().name_ptr, 3);
    }

    #[test]
    fn binary_search_missing_returns_none() {
        let table = FnMetaTable {
            entries: vec![make_entry(100, 1, 2)],
        };
        assert!(table.lookup(999).is_none());
    }

    #[test]
    fn empty_table_lookup_returns_none() {
        let table = FnMetaTable::empty();
        assert!(table.lookup(42).is_none());
    }
}
