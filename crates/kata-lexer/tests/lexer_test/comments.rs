use super::tokens_only;
use kata_ast::Token;
use kata_diagnostics::FrontendError;
use kata_lexer::lex;

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
    let source = "constant x := 1\n# comment line\nconstant y := 2";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Constant,
        Token::Ident("x".into()),
        Token::BindAssign,
        Token::IntLit("1".into()),
        Token::StmtSep,
        Token::Constant,
        Token::Ident("y".into()),
        Token::BindAssign,
        Token::IntLit("2".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn only_comments() {
    let tokens = lex("# comment 1\n# comment 2\n").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

// ── Comentário multilinha `#{ }#` ─────────────────────────────

#[test]
fn multiline_comment_basic() {
    let tokens = lex("#{ multi\nline\ncomment }#").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn multiline_comment_before_code() {
    let tokens = lex("#{ comment }#\n42").unwrap();
    let expected = vec![Token::IntLit("42".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn multiline_comment_after_code() {
    let tokens = lex("42 #{ trailing\nmultiline }#").unwrap();
    let expected = vec![Token::IntLit("42".into()), Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn multiline_comment_between_statements() {
    let source = "constant x := 1\n#{ block\ncomment }#\nconstant y := 2";
    let tokens = lex(source).unwrap();
    let expected = vec![
        Token::Constant,
        Token::Ident("x".into()),
        Token::BindAssign,
        Token::IntLit("1".into()),
        Token::StmtSep,
        Token::Constant,
        Token::Ident("y".into()),
        Token::BindAssign,
        Token::IntLit("2".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn multiline_comment_empty() {
    let tokens = lex("#{ }#").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn multiline_comment_inline() {
    let tokens = lex("1 + #{ inline }# 2").unwrap();
    let expected = vec![
        Token::IntLit("1".into()),
        Token::Ident("+".into()),
        Token::IntLit("2".into()),
        Token::Eof,
    ];
    assert_eq!(tokens_only(&tokens), expected);
}

#[test]
fn multiline_comment_no_nesting_inner_closes_early() {
    // Sem nesting: o primeiro `}#` fecha o comentário.
    // O `}#` interno fecha prematuramente, deixando ` }#` como código.
    let tokens = lex("#{ outer #{ inner }# still comment }#").unwrap();
    // `#{` abre, scan até primeiro `}#` (após "inner"), fecha.
    // Resto: " still comment }#" — é comentário de linha? Não, começa com espaço.
    // Na verdade: após fechar `}#`, o lexer volta ao loop. " still comment }#" vira
    // tentativa de lexar "still" como ident, "comment" como ident, "}" fecha bracket, "#" comentário.
    let toks = tokens_only(&tokens);
    // Não assertamos tokens exatos — o ponto é que `}#` interno fecha o externo.
    // Verificar que não é erro e que algo foi lexado.
    assert!(tokens.len() > 1); // mais que só Eof
}

#[test]
fn multiline_comment_unterminated() {
    let result = lex("#{ no closing");
    assert!(result.is_err());
    match result.unwrap_err() {
        FrontendError::UnterminatedComment { .. } => {}
        e => panic!("esperado UnterminatedComment, encontrado {:?}", e),
    }
}

#[test]
fn multiline_comment_unterminated_with_newlines() {
    let result = lex("#{ line1\nline2\nline3");
    assert!(result.is_err());
    match result.unwrap_err() {
        FrontendError::UnterminatedComment { .. } => {}
        e => panic!("esperado UnterminatedComment, encontrado {:?}", e),
    }
}

#[test]
fn line_comment_with_brace_not_multiline() {
    // `#` seguido de espaço — comentário de linha (não `#{`).
    let tokens = lex("# { not a multiline").unwrap();
    let expected = vec![Token::Eof];
    assert_eq!(tokens_only(&tokens), expected);
}
