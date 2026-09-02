//! Dispatch de FFI — mapeamento de ffi_symbol → função kata-rt.
//!
//! Cada entrada é uma chamada de função Rust direta — sem overhead de
//! C-ABI calling convention. O overhead por operação é um match Rust +
//! function call, não uma `callq` C-ABI.
//!
//! Para aritmética SMI, inlinamos o untag/tag quando possível.

use std::ffi::CString;

use kata_rt as rt;

use crate::value::{Value, decode_smi, encode_smi, f64_to_value, fits_smi, is_smi, value_to_f64};

/// Despacha uma chamada FFI pelo nome do símbolo.
///
/// `args` é slice de Value (i64). `rt_ptr` é ponteiro para o Runtime.
/// `arena` é o handle da fiber arena atual.
pub(crate) fn ffi_dispatch(
    sym: &str,
    args: &[Value],
    rt_ptr: i64,
    arena: i64,
) -> Result<Value, String> {
    match sym {
        // ── Aritmética Int (SMI inline) ──────────────────────
        "kata_rt_bi_add" => {
            let a = args[0];
            let b = args[1];
            if is_smi(a) && is_smi(b) {
                let ra = decode_smi(a);
                let rb = decode_smi(b);
                match ra.checked_add(rb) {
                    Some(result) if fits_smi(result) => return Ok(encode_smi(result)),
                    _ => {}
                }
            }
            Ok(rt::kata_rt_bi_add(a, b))
        }
        "kata_rt_bi_sub" => {
            let a = args[0];
            let b = args[1];
            if is_smi(a) && is_smi(b) {
                let ra = decode_smi(a);
                let rb = decode_smi(b);
                match ra.checked_sub(rb) {
                    Some(result) if fits_smi(result) => return Ok(encode_smi(result)),
                    _ => {}
                }
            }
            Ok(rt::kata_rt_bi_sub(a, b))
        }
        "kata_rt_bi_mul" => {
            let a = args[0];
            let b = args[1];
            if is_smi(a) && is_smi(b) {
                let ra = decode_smi(a);
                let rb = decode_smi(b);
                match ra.checked_mul(rb) {
                    Some(result) if fits_smi(result) => return Ok(encode_smi(result)),
                    _ => {}
                }
            }
            Ok(rt::kata_rt_bi_mul(a, b))
        }
        "kata_rt_bi_div" => Ok(rt::kata_rt_bi_div(args[0], args[1])),

        // ── Comparação Int ───────────────────────────────────
        "kata_rt_bi_eq" => Ok(rt::kata_rt_bi_eq(args[0], args[1])),
        "kata_rt_bi_neq" => Ok(rt::kata_rt_bi_neq(args[0], args[1])),
        "kata_rt_bi_lt" => Ok(rt::kata_rt_bi_lt(args[0], args[1])),
        "kata_rt_bi_gt" => Ok(rt::kata_rt_bi_gt(args[0], args[1])),
        "kata_rt_bi_le" => Ok(rt::kata_rt_bi_le(args[0], args[1])),
        "kata_rt_bi_ge" => Ok(rt::kata_rt_bi_ge(args[0], args[1])),

        // ── Aritmética Float ─────────────────────────────────
        "kata_rt_fadd" => Ok(f64_to_value(rt::kata_rt_fadd(
            value_to_f64(args[0]),
            value_to_f64(args[1]),
        ))),
        "kata_rt_fsub" => Ok(f64_to_value(rt::kata_rt_fsub(
            value_to_f64(args[0]),
            value_to_f64(args[1]),
        ))),
        "kata_rt_fmul" => Ok(f64_to_value(rt::kata_rt_fmul(
            value_to_f64(args[0]),
            value_to_f64(args[1]),
        ))),
        "kata_rt_fdiv" => Ok(f64_to_value(rt::kata_rt_fdiv(
            value_to_f64(args[0]),
            value_to_f64(args[1]),
        ))),

        // ── Comparação Float ─────────────────────────────────
        "kata_rt_fcmp_eq" => Ok(rt::kata_rt_fcmp_eq(
            value_to_f64(args[0]),
            value_to_f64(args[1]),
        )),
        "kata_rt_fcmp_neq" => Ok(rt::kata_rt_fcmp_neq(
            value_to_f64(args[0]),
            value_to_f64(args[1]),
        )),
        "kata_rt_fcmp_lt" => Ok(rt::kata_rt_fcmp_lt(
            value_to_f64(args[0]),
            value_to_f64(args[1]),
        )),
        "kata_rt_fcmp_gt" => Ok(rt::kata_rt_fcmp_gt(
            value_to_f64(args[0]),
            value_to_f64(args[1]),
        )),
        "kata_rt_fcmp_le" => Ok(rt::kata_rt_fcmp_le(
            value_to_f64(args[0]),
            value_to_f64(args[1]),
        )),
        "kata_rt_fcmp_ge" => Ok(rt::kata_rt_fcmp_ge(
            value_to_f64(args[0]),
            value_to_f64(args[1]),
        )),

        // ── Conversões ───────────────────────────────────────
        "kata_rt_int_to_float" => Ok(f64_to_value(rt::kata_rt_int_to_float(decode_smi(args[0])))),
        "kata_rt_float_to_int" => Ok(rt::kata_rt_float_to_int(value_to_f64(args[0]))),
        "kata_rt_int_to_rational" => {
            // kata_rt_int_to_rational chama to_rational(val) que faz is_smi(val)
            // internamente — passar o valor SMI-tagged, não decodificado.
            Ok(unsafe { rt::kata_rt_int_to_rational(args[0]) } as i64)
        }
        "kata_rt_rat_to_float" => {
            let r = args[0] as *const num_rational::BigRational;
            Ok(f64_to_value(unsafe { rt::kata_rt_rat_to_float(r) }))
        }
        "kata_rt_rational_to_int" => {
            let r = args[0] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rational_to_int(r) })
        }
        "kata_rt_rat_from_float" => {
            Ok(unsafe { rt::kata_rt_rat_from_float(value_to_f64(args[0])) } as i64)
        }

        // ── Aritmética Rational ──────────────────────────────
        "kata_rt_rat_add" => {
            let a = args[0] as *const num_rational::BigRational;
            let b = args[1] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_add(a, b) } as i64)
        }
        "kata_rt_rat_sub" => {
            let a = args[0] as *const num_rational::BigRational;
            let b = args[1] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_sub(a, b) } as i64)
        }
        "kata_rt_rat_mul" => {
            let a = args[0] as *const num_rational::BigRational;
            let b = args[1] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_mul(a, b) } as i64)
        }
        "kata_rt_rat_div" => {
            let a = args[0] as *const num_rational::BigRational;
            let b = args[1] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_div(a, b) } as i64)
        }

        // ── Comparação Rational ──────────────────────────────
        "kata_rt_rat_eq" => {
            let a = args[0] as *const num_rational::BigRational;
            let b = args[1] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_eq(a, b) })
        }
        "kata_rt_rat_neq" => {
            let a = args[0] as *const num_rational::BigRational;
            let b = args[1] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_neq(a, b) })
        }
        "kata_rt_rat_lt" => {
            let a = args[0] as *const num_rational::BigRational;
            let b = args[1] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_lt(a, b) })
        }
        "kata_rt_rat_gt" => {
            let a = args[0] as *const num_rational::BigRational;
            let b = args[1] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_gt(a, b) })
        }
        "kata_rt_rat_le" => {
            let a = args[0] as *const num_rational::BigRational;
            let b = args[1] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_le(a, b) })
        }
        "kata_rt_rat_ge" => {
            let a = args[0] as *const num_rational::BigRational;
            let b = args[1] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_ge(a, b) })
        }

        // ── Show / Text ──────────────────────────────────────
        "kata_rt_bi_show" => {
            let ptr = rt::kata_rt_bi_show(args[0]);
            Ok(ptr as i64)
        }
        "kata_rt_float_to_text" => {
            let ptr = rt::kata_rt_float_to_text(value_to_f64(args[0]));
            Ok(ptr as i64)
        }
        "kata_rt_rat_show" => {
            let r = args[0] as *const num_rational::BigRational;
            let ptr = unsafe { rt::kata_rt_rat_show(r) };
            Ok(ptr as i64)
        }
        "kata_rt_bool_to_text" => {
            let ptr = unsafe { rt::kata_rt_bool_to_text(args[0]) };
            Ok(ptr as i64)
        }
        "kata_rt_int_to_text" => {
            let ptr = rt::kata_rt_int_to_text(args[0]);
            Ok(ptr as i64)
        }
        "kata_rt_text_literal" => {
            // Já tratado em eval (TextLit) — se chegar aqui, é erro.
            Err("kata_rt_text_literal não esperado em ffi_dispatch".to_string())
        }
        "kata_rt_string_concat" => {
            let a = args[0] as *const std::os::raw::c_char;
            let b = args[1] as *const std::os::raw::c_char;
            let ptr = unsafe { rt::kata_rt_string_concat(a, b) };
            Ok(ptr as i64)
        }
        "kata_rt_string_eq" => Ok(rt::kata_rt_string_eq(args[0], args[1])),
        "kata_rt_string_starts_with" => Ok(rt::kata_rt_string_starts_with(args[0], args[1])),
        "kata_rt_string_contains" => Ok(rt::kata_rt_string_contains(args[0], args[1])),
        "kata_rt_string_len" => {
            // Retorna SMI — o codegen também retorna SMI
            Ok(encode_smi(unsafe {
                rt::kata_rt_string_len(args[0] as *const std::os::raw::c_char)
            }))
        }

        // ── I/O ──────────────────────────────────────────────
        "kata_rt_print" => {
            let s = args[0] as *const std::os::raw::c_char;
            unsafe { rt::kata_rt_print(s) };
            Ok(0)
        }
        "kata_rt_println" => {
            let s = args[0] as *const std::os::raw::c_char;
            unsafe { rt::kata_rt_println(s) };
            Ok(0)
        }
        "kata_rt_input" => {
            let prompt = args[0] as *const std::os::raw::c_char;
            Ok(unsafe { rt::kata_rt_input(prompt) } as i64)
        }
        "kata_rt_panic" => {
            let s = args[0] as *const std::os::raw::c_char;
            unsafe { rt::kata_rt_panic(s) };
        }
        "kata_rt_sleep" => {
            rt::kata_rt_sleep(args[0]);
            Ok(0)
        }

        // ── List ─────────────────────────────────────────────
        "kata_rt_list_nil" => Ok(rt::kata_rt_list_nil()),
        "kata_rt_list_cons" => Ok(rt::kata_rt_list_cons(args[0], args[1], arena)),
        "kata_rt_list_head" => Ok(rt::kata_rt_list_head(args[0])),
        "kata_rt_list_tail" => Ok(rt::kata_rt_list_tail(args[0])),
        "kata_rt_list_is_empty" => Ok(rt::kata_rt_list_is_empty(args[0])),
        "kata_rt_list_len" => Ok(rt::kata_rt_list_len(args[0])),
        "kata_rt_list_contains" => Ok(rt::kata_rt_list_contains(args[0], args[1])),
        "kata_rt_list_reverse" => Ok(rt::kata_rt_list_reverse(args[0], arena)),
        "kata_rt_list_concat" => Ok(rt::kata_rt_list_concat(args[0], args[1], arena)),
        "kata_rt_list_get_checked" => Ok(rt::kata_rt_list_get_checked(args[0], args[1])),

        // ── Array ────────────────────────────────────────────
        "kata_rt_array_alloc" => Ok(rt::kata_rt_array_alloc(args[0], arena)),
        "kata_rt_array_len" => Ok(rt::kata_rt_array_len(args[0])),
        "kata_rt_array_get" => Ok(rt::kata_rt_array_get(args[0], args[1])),
        "kata_rt_array_set" => {
            rt::kata_rt_array_set(args[0], args[1], args[2]);
            Ok(0)
        }
        "kata_rt_array_get_checked" => Ok(rt::kata_rt_array_get_checked(args[0], args[1])),
        "kata_rt_array_contains" => Ok(rt::kata_rt_array_contains(args[0], args[1])),

        // ── Arena ────────────────────────────────────────────
        "kata_rt_arena_alloc" => Ok(rt::kata_rt_arena_alloc(rt_ptr, args[0], args[1])),
        "kata_rt_get_root_arena_handle" => Ok(rt::kata_rt_get_root_arena_handle(rt_ptr)),

        // ── Sum (variantes de enum) ──────────────────────────
        "kata_rt_store_sum_result" => Ok(rt::kata_rt_store_sum_result(args[0], args[1], arena)),
        "kata_rt_sum_tag_int" => Ok(rt::kata_rt_sum_tag_int(args[0])),

        // ── Zero predicates ──────────────────────────────────
        "kata_rt_bi_zero" => Ok(rt::kata_rt_bi_zero(args[0])),
        "kata_rt_fzero" => Ok(f64_to_value(rt::kata_rt_fzero(value_to_f64(args[0])))),
        "kata_rt_rat_zero" => {
            let r = args[0] as *const num_rational::BigRational;
            Ok(unsafe { rt::kata_rt_rat_zero(r) } as i64)
        }

        // ── Math ─────────────────────────────────────────────
        "kata_rt_gcd" => Ok(rt::kata_rt_gcd(args[0], args[1])),
        "kata_rt_lcm" => Ok(rt::kata_rt_lcm(args[0], args[1])),
        "kata_rt_pow" => Ok(rt::kata_rt_pow(args[0], args[1])),
        "kata_rt_signum" => Ok(rt::kata_rt_signum(args[0])),
        "kata_rt_floor" => Ok(rt::kata_rt_floor(value_to_f64(args[0]))),
        "kata_rt_ceil" => Ok(rt::kata_rt_ceil(value_to_f64(args[0]))),
        "kata_rt_sqrt" => Ok(f64_to_value(rt::kata_rt_sqrt(value_to_f64(args[0])))),
        "kata_rt_sin" => Ok(f64_to_value(rt::kata_rt_sin(value_to_f64(args[0])))),
        "kata_rt_cos" => Ok(f64_to_value(rt::kata_rt_cos(value_to_f64(args[0])))),
        "kata_rt_tan" => Ok(f64_to_value(rt::kata_rt_tan(value_to_f64(args[0])))),
        "kata_rt_exp" => Ok(f64_to_value(rt::kata_rt_exp(value_to_f64(args[0])))),
        "kata_rt_log" => Ok(f64_to_value(rt::kata_rt_log(value_to_f64(args[0])))),
        "kata_rt_log2" => Ok(f64_to_value(rt::kata_rt_log2(value_to_f64(args[0])))),
        "kata_rt_log10" => Ok(f64_to_value(rt::kata_rt_log10(value_to_f64(args[0])))),
        "kata_rt_rand" => Ok(f64_to_value(rt::kata_rt_rand())),
        "kata_rt_rand_int" => Ok(rt::kata_rt_rand_int(args[0], args[1])),

        // ── Byte ─────────────────────────────────────────────
        "kata_rt_byte_and" => Ok(rt::kata_rt_byte_and(args[0], args[1])),
        "kata_rt_byte_or" => Ok(rt::kata_rt_byte_or(args[0], args[1])),
        "kata_rt_byte_xor" => Ok(rt::kata_rt_byte_xor(args[0], args[1])),
        "kata_rt_byte_not" => Ok(rt::kata_rt_byte_not(args[0])),
        "kata_rt_byte_shl" => Ok(rt::kata_rt_byte_shl(args[0], args[1])),
        "kata_rt_byte_shr" => Ok(rt::kata_rt_byte_shr(args[0], args[1])),
        "kata_rt_byte_to_int" => Ok(rt::kata_rt_byte_to_int(args[0])),
        "kata_rt_int_to_byte" => Ok(rt::kata_rt_int_to_byte(args[0])),

        // ── Text slice ───────────────────────────────────────
        "kata_rt_text_len" => Ok(encode_smi(unsafe { rt::kata_rt_text_len(args[0]) })),
        "kata_rt_text_at" => Ok(unsafe { rt::kata_rt_text_at(args[0], args[1], arena) }),

        // ── Rat literal (para ascription de Rational) ────────
        "kata_rt_rat_literal" => {
            // args[0] = numerador (SMI), args[1] = denominador (SMI)
            // kata_rt_rat_literal espera (s: *const c_char, len: i64) —
            // formata como "num/den" e passa como C string.
            let num = decode_smi(args[0]);
            let den = decode_smi(args[1]);
            let text = format!("{num}/{den}");
            let len = text.len() as i64;
            let cstr = CString::new(text).unwrap();
            let ptr = cstr.into_raw();
            let result = unsafe { rt::kata_rt_rat_literal(ptr, len) };
            unsafe { drop(CString::from_raw(ptr)) };
            Ok(result as i64)
        }

        // ── Tag int from str (BigInt literal) ────────────────
        "kata_rt_tag_int_from_str" => {
            // args[0] = ptr, args[1] = len
            let ptr = args[0] as *const std::os::raw::c_char;
            let len = args[1];
            Ok(rt::kata_rt_tag_int_from_str(ptr, len))
        }

        // ── Text to int/float (conversões) ───────────────────
        "kata_rt_text_to_int" => {
            Ok(unsafe { rt::kata_rt_text_to_int(args[0] as *const std::os::raw::c_char) })
        }
        "kata_rt_try_int" => {
            Ok(unsafe { rt::kata_rt_try_int(args[0] as *const std::os::raw::c_char) })
        }
        "kata_rt_text_to_float" => Ok(f64_to_value(unsafe {
            rt::kata_rt_text_to_float(args[0] as *const std::os::raw::c_char)
        })),
        "kata_rt_try_float" => {
            Ok(unsafe { rt::kata_rt_try_float(args[0] as *const std::os::raw::c_char) })
        }

        // ── Bi to rational ───────────────────────────────────
        "kata_rt_bi_to_rational" => {
            let ptr = rt::kata_rt_bi_to_rational(args[0]);
            Ok(ptr as i64)
        }

        // ── Text replace ─────────────────────────────────────
        "kata_rt_text_replace" => {
            let ptr = unsafe {
                rt::kata_rt_text_replace(
                    args[0] as *const std::os::raw::c_char,
                    args[1] as *const std::os::raw::c_char,
                    args[2] as *const std::os::raw::c_char,
                )
            };
            Ok(ptr as i64)
        }
        "kata_rt_text_replace_first" => {
            let ptr = unsafe {
                rt::kata_rt_text_replace_first(
                    args[0] as *const std::os::raw::c_char,
                    args[1] as *const std::os::raw::c_char,
                )
            };
            Ok(ptr as i64)
        }

        // ── Arc (CaptureBox) ─────────────────────────────────
        "kata_rt_alloc_arc" => Ok(rt::kata_rt_alloc_arc(
            rt_ptr, args[0], args[1], args[2], arena,
        )),
        "kata_rt_incref" => Ok(rt::kata_rt_incref(args[0])),
        "kata_rt_decref" => Ok(rt::kata_rt_decref(rt_ptr, args[0])),
        "kata_rt_arc_fn_ptr" => Ok(rt::kata_rt_arc_fn_ptr(args[0])),

        // ── Timer ────────────────────────────────────────────
        "kata_rt_timer_now" => Ok(rt::kata_rt_timer_now()),

        // ── Hash ─────────────────────────────────────────────
        "kata_rt_hash_int" => Ok(rt::kata_rt_hash_int(args[0])),
        "kata_rt_hash_text" => Ok(rt::kata_rt_hash_text(args[0])),
        "kata_rt_hash_rational" => Ok(rt::kata_rt_hash_rational(args[0])),

        // ── Set ──────────────────────────────────────────────
        "kata_rt_set_empty" => Ok(rt::kata_rt_set_empty(arena)),
        "kata_rt_set_insert" => Ok(rt::kata_rt_set_insert(
            args[0], args[1], args[2], args[3], arena,
        )),
        "kata_rt_set_contains" => Ok(rt::kata_rt_set_contains(args[0], args[1], args[2], args[3])),
        "kata_rt_set_len" => Ok(rt::kata_rt_set_len(args[0])),
        "kata_rt_set_union" => Ok(rt::kata_rt_set_union(args[0], args[1], args[2], arena)),
        "kata_rt_set_intersection" => Ok(rt::kata_rt_set_intersection(
            args[0], args[1], args[2], arena,
        )),
        "kata_rt_set_difference" => {
            Ok(rt::kata_rt_set_difference(args[0], args[1], args[2], arena))
        }

        // ── Dict ─────────────────────────────────────────────
        "kata_rt_dict_empty" => Ok(rt::kata_rt_dict_empty(arena)),
        "kata_rt_dict_insert" => Ok(rt::kata_rt_dict_insert(
            args[0], args[1], args[2], args[3], args[4], arena,
        )),
        "kata_rt_dict_get_checked" => Ok(rt::kata_rt_dict_get_checked(
            args[0], args[1], args[2], args[3], arena,
        )),
        "kata_rt_dict_contains" => Ok(rt::kata_rt_dict_contains(
            args[0], args[1], args[2], args[3],
        )),
        "kata_rt_dict_len" => Ok(rt::kata_rt_dict_len(args[0])),
        "kata_rt_dict_merge" => Ok(rt::kata_rt_dict_merge(args[0], args[1], args[2], arena)),

        // ── Range ────────────────────────────────────────────
        "kata_rt_range_alloc" => Ok(rt::kata_rt_range_alloc(arena)),

        // ── Snapshot ─────────────────────────────────────────
        "kata_rt_get_snapshot" => Ok(rt::kata_rt_get_snapshot(args[0])),

        // ── Arena create/destroy ─────────────────────────────
        "kata_rt_arena_create" => Ok(rt::kata_rt_arena_create(rt_ptr)),
        "kata_rt_arena_create_tracked" => Ok(rt::kata_rt_arena_create_tracked(rt_ptr)),
        "kata_rt_arena_destroy" => {
            rt::kata_rt_arena_destroy(rt_ptr, args[0]);
            Ok(0)
        }

        // ── File I/O ─────────────────────────────────────────
        "kata_rt_file_open" => {
            let path = args[0] as *const std::os::raw::c_char;
            let mode = args[1];
            Ok(unsafe { rt::kata_rt_file_open(path, mode, arena) })
        }
        "kata_rt_file_read" => Ok(unsafe { rt::kata_rt_file_read(args[0]) }),
        "kata_rt_file_readline" => Ok(unsafe { rt::kata_rt_file_readline(args[0]) }),
        "kata_rt_file_write_text" => Ok(unsafe { rt::kata_rt_file_write_text(args[0], args[1]) }),
        "kata_rt_file_write_bytes" => Ok(unsafe { rt::kata_rt_file_write_bytes(args[0], args[1]) }),
        "kata_rt_file_close" => {
            unsafe { rt::kata_rt_file_close(args[0]) };
            Ok(0)
        }
        "kata_rt_stdin" => Ok(rt::kata_rt_stdin()),
        "kata_rt_stdout" => Ok(rt::kata_rt_stdout()),
        "kata_rt_stderr" => Ok(rt::kata_rt_stderr()),

        // ── Scheduler (básico) ───────────────────────────────
        "kata_rt_yield" => {
            rt::kata_rt_yield();
            Ok(0)
        }
        "kata_rt_yield_check" => {
            rt::kata_rt_yield_check(rt_ptr);
            Ok(0)
        }

        // ── Log (@log, log!, log_recv!) ───────────────────────
        "kata_rt_log_publish" => Ok(rt::kata_rt_log_publish(args[0], args[1], args[2], args[3])),
        "kata_rt_log_publish_default" => Ok(rt::kata_rt_log_publish_default(args[0], args[1])),
        "kata_rt_log_publish_topic" => Ok(rt::kata_rt_log_publish_topic(args[0], args[1], args[2])),
        "kata_rt_log_publish_full" => Ok(rt::kata_rt_log_publish_full(
            args[0], args[1], args[2], args[3],
        )),
        "kata_rt_log_recv" => Ok(rt::kata_rt_log_recv(args[0])),
        "kata_rt_log_config" => {
            rt::kata_rt_log_config(args[0], args[1], args[2]);
            Ok(0)
        }

        // ── Show sintetizado para tipos compostos ────────────
        // Estas são funções geradas pelo codegen (__kata_show__Type).
        // O interpretador não tem codegen — precisamos implementar
        // show para tipos compostos diretamente (Fase 2).
        sym if sym.starts_with("__kata_show__") => {
            // Placeholder — o show real é interceptado em eval.rs
            // (ffi_dispatch não tem acesso ao Ty do valor)
            let cstr =
                CString::new(format!("<{}>", sym)).unwrap_or_else(|_| CString::new("?").unwrap());
            Ok(cstr.into_raw() as i64)
        }

        // ── Recursion depth ──────────────────────────────────
        "kata_rt_depth_set_limit" => {
            // args[0] é SMI-tagged — decodificar antes de passar ao Runtime.
            let limit = decode_smi(args[0]);
            rt::kata_rt_depth_set_limit(rt_ptr, limit);
            Ok(0) // Unit = SMI 0
        }

        // ── Não implementado ─────────────────────────────────
        _ => Err(format!("FFI não implementado no interpretador: {sym}")),
    }
}
