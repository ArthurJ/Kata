use kata_ast::{Token, TokenWithSpan};
use kata_diagnostics::FrontendError;
use kata_lexer::lex;

/// Extrai apenas os tokens (sem span) de uma lista de TokenWithSpan.
fn tokens_only(tws: &[TokenWithSpan]) -> Vec<Token> {
    tws.iter().map(|t| t.token.clone()).collect()
}

// ── Aritmética simples ─────────────────────────────────────────

#[test]
fn simple_arithmetic_prefix() {
    let tokens = lex("+ 1 2").unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn signed_number_no_space() {
    // `+1` sem espaço = número positivo
    let tokens = lex("+1").unwrap();
    let expected = vec![Token::IntLit("+1".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn negative_number_no_space() {
    let tokens = lex("-42").unwrap();
    let expected = vec![Token::IntLit("-42".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn operators_as_identifiers() {
    let tokens = lex("+ - * / < > =").unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::Ident("-".into()),
        Token::Ident("*".into()),
        Token::Ident("/".into()),
        Token::Ident("<".into()),
        Token::Ident(">".into()),
        Token::Ident("=".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn dollar_is_ident() {
    let tokens = lex("$").unwrap();
    let expected = vec![Token::Ident("$".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Números ────────────────────────────────────────────────────

#[test]
fn int_decimal() {
    let tokens = lex("42").unwrap();
    let expected = vec![Token::IntLit("42".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_hex() {
    let tokens = lex("0xFF").unwrap();
    let expected = vec![Token::IntLit("0xFF".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_octal() {
    let tokens = lex("0o77").unwrap();
    let expected = vec![Token::IntLit("0o77".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_binary() {
    let tokens = lex("0b1010").unwrap();
    let expected = vec![Token::IntLit("0b1010".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_explicit_decimal() {
    let tokens = lex("0d255").unwrap();
    let expected = vec![Token::IntLit("0d255".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_underscore_separator() {
    let tokens = lex("1_000").unwrap();
    // underscore é descartado léxicamente
    let expected = vec![Token::IntLit("1000".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn int_zero() {
    let tokens = lex("0").unwrap();
    let expected = vec![Token::IntLit("0".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Float ──────────────────────────────────────────────────────

#[test]
fn float_decimal() {
    let tokens = lex("3.14").unwrap();
    let expected = vec![Token::FloatLit("3.14".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_scientific() {
    let tokens = lex("1.5e10").unwrap();
    let expected = vec![Token::FloatLit("1.5e10".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_scientific_negative_exp() {
    let tokens = lex("1.5E-10").unwrap();
    let expected = vec![Token::FloatLit("1.5E-10".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_scientific_no_point() {
    let tokens = lex("1e10").unwrap();
    let expected = vec![Token::FloatLit("1e10".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn float_with_underscore() {
    let tokens = lex("1_000.5").unwrap();
    let expected = vec![Token::FloatLit("1000.5".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Strings ────────────────────────────────────────────────────

#[test]
fn string_double_quotes() {
    let tokens = lex("\"hello\"").unwrap();
    let expected = vec![Token::TextLit("hello".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_single_quotes() {
    let tokens = lex("'world'").unwrap();
    let expected = vec![Token::TextLit("world".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_escape_newline() {
    let tokens = lex("\"hello\\nworld\"").unwrap();
    let expected = vec![Token::TextLit("hello\nworld".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_escape_tab() {
    let tokens = lex("\"a\\tb\"").unwrap();
    let expected = vec![Token::TextLit("a\tb".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_escape_backslash() {
    let tokens = lex("\"a\\\\b\"").unwrap();
    let expected = vec![Token::TextLit("a\\b".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_escape_quote() {
    let tokens = lex("\"say \\\"hi\\\"\"").unwrap();
    let expected = vec![Token::TextLit("say \"hi\"".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_single_quote_escape() {
    let tokens = lex("'it\\'s'").unwrap();
    let expected = vec![Token::TextLit("it's".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_triple_quotes_raw() {
    let tokens = lex("\"\"\"raw\\ntext\"\"\"").unwrap();
    // Triplas são raw — \n é literal, não escape
    let expected = vec![Token::TextLit("raw\\ntext".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_triple_quotes_multiline() {
    let tokens = lex("\"\"\"line1\nline2\"\"\"").unwrap();
    let expected = vec![Token::TextLit("line1\nline2".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn string_unknown_escape_preserved() {
    let tokens = lex("\"\\x\"").unwrap();
    let expected = vec![Token::TextLit("\\x".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Palavras-chave ─────────────────────────────────────────────

#[test]
fn keyword_let() {
    let tokens = lex("let").unwrap();
    let expected = vec![Token::Let, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_var() {
    let tokens = lex("var").unwrap();
    let expected = vec![Token::Var, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_data() {
    let tokens = lex("data").unwrap();
    let expected = vec![Token::Data, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_enum() {
    let tokens = lex("enum").unwrap();
    let expected = vec![Token::Enum, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_lambda() {
    let tokens = lex("lambda").unwrap();
    let expected = vec![Token::Lambda, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_import() {
    let tokens = lex("import").unwrap();
    let expected = vec![Token::Import, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_export() {
    let tokens = lex("export").unwrap();
    let expected = vec![Token::Export, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_as() {
    let tokens = lex("as").unwrap();
    let expected = vec![Token::As, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_interface() {
    let tokens = lex("interface").unwrap();
    let expected = vec![Token::Interface, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_implements() {
    let tokens = lex("implements").unwrap();
    let expected = vec![Token::Implements, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_with() {
    let tokens = lex("with").unwrap();
    let expected = vec![Token::With, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_match() {
    let tokens = lex("match").unwrap();
    let expected = vec![Token::Match, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_return() {
    let tokens = lex("return").unwrap();
    let expected = vec![Token::Return, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_otherwise() {
    let tokens = lex("otherwise").unwrap();
    let expected = vec![Token::Otherwise, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_alias() {
    let tokens = lex("alias").unwrap();
    let expected = vec![Token::Alias, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn keyword_action() {
    let tokens = lex("action").unwrap();
    let expected = vec![Token::Action, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn lambda_unicode() {
    let tokens = lex("λ").unwrap();
    let expected = vec![Token::Lambda, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn identifier_not_keyword() {
    let tokens = lex("myVar").unwrap();
    let expected = vec![Token::Ident("myVar".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn identifier_camelcase() {
    let tokens = lex("MyType").unwrap();
    // CamelCase é identificador normal em Kata5 (sem UpperIdent)
    let expected = vec![Token::Ident("MyType".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Pontuação e operadores ─────────────────────────────────────

#[test]
fn punct_bind_assign() {
    let tokens = lex(":=").unwrap();
    let expected = vec![Token::BindAssign, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_double_colon() {
    let tokens = lex("::").unwrap();
    let expected = vec![Token::DoubleColon, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_fat_arrow() {
    let tokens = lex("=>").unwrap();
    let expected = vec![Token::FatArrow, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_thin_arrow() {
    let tokens = lex("->").unwrap();
    let expected = vec![Token::ThinArrow, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_pipe() {
    let tokens = lex("|").unwrap();
    let expected = vec![Token::Pipe, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_pipe_forward() {
    let tokens = lex("|>").unwrap();
    let expected = vec![Token::PipeForward, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_question() {
    let tokens = lex("?").unwrap();
    let expected = vec![Token::Question, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_bang() {
    let tokens = lex("!").unwrap();
    let expected = vec![Token::Bang, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_parens() {
    let tokens = lex("()").unwrap();
    let expected = vec![Token::LParen, Token::RParen, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_brackets() {
    let tokens = lex("[]").unwrap();
    let expected = vec![Token::LBracket, Token::RBracket, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_braces() {
    let tokens = lex("{}").unwrap();
    let expected = vec![Token::LBrace, Token::RBrace, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_comma() {
    let tokens = lex(",").unwrap();
    let expected = vec![Token::Comma, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_dot() {
    let tokens = lex(".").unwrap();
    let expected = vec![Token::Dot, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_semicolon() {
    let tokens = lex(";").unwrap();
    let expected = vec![Token::Semicolon, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_colon() {
    let tokens = lex(":").unwrap();
    let expected = vec![Token::Colon, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn punct_at() {
    let tokens = lex("@").unwrap();
    let expected = vec![Token::At, Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn equal_alone_is_ident() {
    let tokens = lex("=").unwrap();
    let expected = vec![Token::Ident("=".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn minus_alone_is_ident() {
    let tokens = lex("-").unwrap();
    let expected = vec![Token::Ident("-".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Comentários ────────────────────────────────────────────────

#[test]
fn comment_line() {
    let tokens = lex("# this is a comment").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn comment_after_code() {
    let tokens = lex("42 # trailing comment").unwrap();
    let expected = vec![Token::IntLit("42".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn comment_between_statements() {
    let source = "let x := 1\n# comment line\nlet y := 2";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Let,
        Token::Ident("x".into()),
        Token::BindAssign,
        Token::IntLit("1".into()),
        Token::StmtSep,
        Token::Let,
        Token::Ident("y".into()),
        Token::BindAssign,
        Token::IntLit("2".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Indentação ─────────────────────────────────────────────────

#[test]
fn indent_simple_block() {
    let source = "let x := 1\n    + 1 2";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Let,
        Token::Ident("x".into()),
        Token::BindAssign,
        Token::IntLit("1".into()),
        Token::Indent,
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::Dedent,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn indent_no_change_no_stmtsep_at_eof() {
    // Ultima linha não gera StmtSep antes de EOF
    let tokens = lex("+ 1 2").unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn stmtsep_between_statements() {
    let source = "+ 1 2\n+ 3 4";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::StmtSep,
        Token::Ident("+".into()),
        Token::IntLit("3".into()),
        Token::IntLit("4".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn indent_dedent_block() {
    let source = "let x := 1\n    + 1 2\n+ 3 4";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Let,
        Token::Ident("x".into()),
        Token::BindAssign,
        Token::IntLit("1".into()),
        Token::Indent,
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::Dedent,
        Token::Ident("+".into()),
        Token::IntLit("3".into()),
        Token::IntLit("4".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn blank_lines_ignored() {
    let source = "+ 1 2\n\n\n+ 3 4";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::StmtSep,
        Token::Ident("+".into()),
        Token::IntLit("3".into()),
        Token::IntLit("4".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn newline_in_parens_no_stmtsep() {
    let source = "(+ 1\n2)";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::LParen,
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::IntLit("2".into()),
        Token::RParen,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn enum_block_indent() {
    let source = "enum Boolean\n    True\n    False";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Enum,
        Token::Ident("Boolean".into()),
        Token::Indent,
        Token::Ident("True".into()),
        Token::StmtSep,
        Token::Ident("False".into()),
        Token::Dedent,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Spans ──────────────────────────────────────────────────────

#[test]
fn span_of_first_token() {
    let tokens = lex("+ 1 2").unwrap();
    let first = &tokens[0];
    assert_eq!(first.token, Token::Ident("+".into()));
    assert_eq!(first.span.offset, 0);
    assert_eq!(first.span.line, 1);
    assert_eq!(first.span.col, 1);
    assert_eq!(first.span.len, 1);
}

#[test]
fn span_of_second_token() {
    let tokens = lex("+ 1 2").unwrap();
    let second = &tokens[1];
    assert_eq!(second.token, Token::IntLit("1".into()));
    assert_eq!(second.span.offset, 2);
    assert_eq!(second.span.line, 1);
    assert_eq!(second.span.col, 3);
    assert_eq!(second.span.len, 1);
}

#[test]
fn span_multiline_second_line() {
    let source = "+ 1 2\n+ 3 4";
    let tokens = lex(source).unwrap();
    // Token `+` na segunda linha (após StmtSep)
    let plus_idx = tokens
        .iter()
        .position(|t| t.token == Token::Ident("+".into()) && t.span.line == 2)
        .expect("encontrou + na linha 2");
    let plus = &tokens[plus_idx];
    assert_eq!(plus.span.line, 2);
    assert_eq!(plus.span.col, 1);
    assert_eq!(plus.span.offset, 6); // "+ 1 2\n" = 6 bytes
    assert_eq!(plus.span.len, 1);
}

// ── Casos compostos ────────────────────────────────────────────

#[test]
fn let_binding() {
    let tokens = lex("let x := 42").unwrap();
    let expected = vec![
        Token::Let,
        Token::Ident("x".into()),
        Token::BindAssign,
        Token::IntLit("42".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn ffi_directive_and_signature() {
    let source = "@ffi(\"kata_rt_bi_add\")\n+ :: Int Int => Int";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::At,
        Token::Ident("ffi".into()),
        Token::LParen,
        Token::TextLit("kata_rt_bi_add".into()),
        Token::RParen,
        Token::StmtSep,
        Token::Ident("+".into()),
        Token::DoubleColon,
        Token::Ident("Int".into()),
        Token::Ident("Int".into()),
        Token::FatArrow,
        Token::Ident("Int".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn data_decl_opaque() {
    let source = "data Int ()";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Data,
        Token::Ident("Int".into()),
        Token::LParen,
        Token::RParen,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn complex_expression() {
    // `+ 1 (* 2 3)` — aplicação prefixa com parênteses
    let tokens = lex("+ 1 (* 2 3)").unwrap();
    let expected = vec![
        Token::Ident("+".into()),
        Token::IntLit("1".into()),
        Token::LParen,
        Token::Ident("*".into()),
        Token::IntLit("2".into()),
        Token::IntLit("3".into()),
        Token::RParen,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Erros ──────────────────────────────────────────────────────

#[test]
fn error_unterminated_string() {
    let result = lex("\"hello");
    assert!(result.is_err());
    match result.unwrap_err() {
        FrontendError::UnterminatedString { .. } => {}
        e => panic!("esperado UnterminatedString, encontrado {:?}", e),
    }
}

#[test]
fn error_unterminated_single_quote_string() {
    let result = lex("'hello");
    assert!(result.is_err());
    match result.unwrap_err() {
        FrontendError::UnterminatedString { .. } => {}
        e => panic!("esperado UnterminatedString, encontrado {:?}", e),
    }
}

#[test]
fn error_unterminated_triple_string() {
    let result = lex("\"\"\"hello");
    assert!(result.is_err());
    match result.unwrap_err() {
        FrontendError::UnterminatedString { .. } => {}
        e => panic!("esperado UnterminatedString, encontrado {:?}", e),
    }
}

#[test]
fn error_invalid_number_no_digits_after_prefix() {
    let result = lex("0x");
    assert!(result.is_err());
    match result.unwrap_err() {
        FrontendError::InvalidNumber { .. } => {}
        e => panic!("esperado InvalidNumber, encontrado {:?}", e),
    }
}

#[test]
fn error_invalid_float_exp_no_digits() {
    let result = lex("1.5e");
    assert!(result.is_err());
    match result.unwrap_err() {
        FrontendError::InvalidNumber { .. } => {}
        e => panic!("esperado InvalidNumber, encontrado {:?}", e),
    }
}

#[test]
fn empty_source() {
    let tokens = lex("").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn only_whitespace() {
    let tokens = lex("   \n  \n  ").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn only_comments() {
    let tokens = lex("# comment 1\n# comment 2\n").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn nested_indent_levels() {
    let source = "a\n    b\n        c\n    d";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Ident("a".into()),
        Token::Indent,
        Token::Ident("b".into()),
        Token::Indent,
        Token::Ident("c".into()),
        Token::Dedent,
        Token::Ident("d".into()),
        Token::Dedent,
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}
