//! Prelude hardcoded — injetado pelo module loader.
//!
//! Fio 10 substitui isto por carregamento de `stdlib/core.kata` do filesystem.
//! Por agora, o prelude é uma string constante que o resolution parseia
//! e injeta no TypeEnv + DispatchTable.

/// Código fonte do prelude hardcoded.
/// Define tipos opacos (Int, Float, Text, Rational), Boolean, e operadores via @ffi.
pub const PRELUDE_SOURCE: &str = r#"# Tipos opacos (anchored via @ffi)
@ffi("i64")
data Int ()

@ffi("f64")
data Float ()

@ffi("kata_rt_string")
data Text ()

@ffi("kata_rt_rat")
data Rational ()

# Boolean
enum Boolean
    False
    True

# Aritmética Int (BigInt/SMI no runtime)
@ffi("kata_rt_bi_add")
@associative(0)
+ :: Int Int => Int

@ffi("kata_rt_bi_sub")
- :: Int Int => Int

@ffi("kata_rt_bi_mul")
@associative(1)
* :: Int Int => Int

@ffi("kata_rt_bi_div")
/ :: Int Int => Int

# Comparação Int
@ffi("kata_rt_bi_eq")
= :: Int Int => Boolean

@ffi("kata_rt_bi_lt")
< :: Int Int => Boolean

@ffi("kata_rt_bi_gt")
> :: Int Int => Boolean

# Aritmética Float
@ffi("kata_rt_fadd")
+ :: Float Float => Float

@ffi("kata_rt_fsub")
- :: Float Float => Float

@ffi("kata_rt_fmul")
* :: Float Float => Float

@ffi("kata_rt_fdiv")
/ :: Float Float => Float

# Comparação Float
@ffi("kata_rt_fcmp_eq")
= :: Float Float => Boolean

@ffi("kata_rt_fcmp_lt")
< :: Float Float => Boolean

@ffi("kata_rt_fcmp_gt")
> :: Float Float => Boolean

# Aritmética Rational
@ffi("kata_rt_rat_add")
@associative(0)
+ :: Rational Rational => Rational

@ffi("kata_rt_rat_sub")
- :: Rational Rational => Rational

@ffi("kata_rt_rat_mul")
@associative(1)
* :: Rational Rational => Rational

@ffi("kata_rt_rat_div")
/ :: Rational Rational => Rational

# Comparação Rational
@ffi("kata_rt_rat_eq")
= :: Rational Rational => Boolean

@ffi("kata_rt_rat_lt")
< :: Rational Rational => Boolean

@ffi("kata_rt_rat_gt")
> :: Rational Rational => Boolean

# Conversões
@ffi("kata_rt_rat_to_float")
to_float :: Rational => Float

@ffi("kata_rt_rat_from_float")
from_float :: Float => Rational

@ffi("kata_rt_int_to_rational")
from_int :: Int => Rational

# I/O
@ffi("kata_rt_print")
echo :: (Text) => Unit

# Show (conversão para Text)
@ffi("kata_rt_bi_show")
show :: Int => Text

@ffi("kata_rt_rat_show")
show :: Rational => Text
"#;
