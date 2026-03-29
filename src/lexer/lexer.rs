use super::token::Token;
use chumsky::prelude::*;

pub type Span = std::ops::Range<usize>;

#[derive(Clone, Debug, PartialEq)]
enum RawToken {
    Tok(Token),
    Spaces(usize),
    Ignore,
}

/// The base lexer that parses characters into a flat list of RawTokens and Newlines
fn raw_lexer() -> impl Parser<char, Vec<(RawToken, Span)>, Error = Simple<char>> {
    let digit_with_underscores = |radix: u32| {
        filter(move |c: &char| c.is_digit(radix))
            .then(filter(move |c: &char| c.is_digit(radix) || *c == '_').repeated().collect::<String>())
            .map(|(first, rest)| {
                let mut s = first.to_string();
                s.push_str(&rest.replace('_', ""));
                s
            })
    };

    let bin_num = just("0b").ignore_then(digit_with_underscores(2)).map(|s| format!("0b{}", s));
    let oct_num = just("0o").ignore_then(digit_with_underscores(8)).map(|s| format!("0o{}", s));
    let hex_num = just("0x").ignore_then(digit_with_underscores(16)).map(|s| format!("0x{}", s));

    let dec_num = digit_with_underscores(10)
        .then(just('.').ignore_then(digit_with_underscores(10)).or_not())
        .map(|(int_part, frac_part)| match frac_part {
            Some(frac) => RawToken::Tok(Token::Float(format!("{}.{}", int_part, frac))),
            None => RawToken::Tok(Token::Int(int_part)),
        });

    let num = choice((
        bin_num.map(|s| RawToken::Tok(Token::Int(s))),
        oct_num.map(|s| RawToken::Tok(Token::Int(s))),
        hex_num.map(|s| RawToken::Tok(Token::Int(s))),
        dec_num,
    ));

    let string = just('"')
        .ignore_then(filter(|c| *c != '"').repeated().collect::<String>())
        .then_ignore(just('"'))
        .map(|s| RawToken::Tok(Token::String(s)));

    let op = choice((
        just("!>").to(Token::ChannelSend),
        just("<!?").to(Token::ChannelRecvNonBlock),
        just("<!").to(Token::ChannelRecv),
        just("::").to(Token::DoubleColon),
        just("->").to(Token::Arrow),
        just("=>").to(Token::FatArrow),
        just("(").to(Token::LParen),
        just(")").to(Token::RParen),
        just("[").to(Token::LBracket),
        just("]").to(Token::RBracket),
        just("{").to(Token::LBrace),
        just("}").to(Token::RBrace),
        just("|").to(Token::Pipe),
        just(":").to(Token::Colon),
        just("$").to(Token::Dollar),
        just("...").to(Token::Ellipsis),
        just(".").to(Token::Dot),
        just("?").to(Token::Question),
        just(";").to(Token::Semicolon),
        just(",").to(Token::Comma),
        just("&").to(Token::Ampersand),
        just("_").to(Token::Hole),
    ))
    .map(RawToken::Tok);

    let ident_chars = filter(|c: &char| c.is_alphanumeric() || *c == '_')
        .repeated()
        .at_least(1)
        .collect::<String>();

    let alphanumeric_ident = ident_chars
        .then(just('!').or_not()) // capture trailing !
        .map(|(s, bang)| {
            match s.as_str() {
                "data" => RawToken::Tok(Token::Data),
                "enum" => RawToken::Tok(Token::Enum),
                "interface" => RawToken::Tok(Token::Interface),
                "implements" => RawToken::Tok(Token::Implements),
                "export" => RawToken::Tok(Token::Export),
                "import" => RawToken::Tok(Token::Import),
                "lambda" => RawToken::Tok(Token::Lambda),
                "action" => RawToken::Tok(Token::Action),
                "match" => RawToken::Tok(Token::Match),
                "loop" => RawToken::Tok(Token::Loop),
                "for" => RawToken::Tok(Token::For),
                "in" => RawToken::Tok(Token::In),
                "let" => RawToken::Tok(Token::Let),
                "var" => RawToken::Tok(Token::Var),
                "break" => RawToken::Tok(Token::Break),
                "continue" => RawToken::Tok(Token::Continue),
                "with" => RawToken::Tok(Token::With),
                "as" => RawToken::Tok(Token::As),
                "otherwise" => RawToken::Tok(Token::Otherwise),
                "alias" => RawToken::Tok(Token::Alias),
                _ => {
                    let chars: Vec<char> = s.chars().collect();
                    if chars[0].is_uppercase() {
                        if chars.len() == 1 {
                            RawToken::Tok(Token::TypeVar(s))
                        } else if chars.iter().all(|c| c.is_uppercase() || *c == '_') {
                            RawToken::Tok(Token::InterfaceID(s))
                        } else {
                            RawToken::Tok(Token::TypeID(s))
                        }
                    } else if bang.is_some() {
                        RawToken::Tok(Token::ActionIdent(s))
                    } else {
                        RawToken::Tok(Token::Ident(s))
                    }
                }
            }
        });

    let symbolic_ident = one_of("+-*/=<>!\\@^%~?")
        .repeated()
        .at_least(1)
        .collect::<String>()
        .map(|s| RawToken::Tok(Token::Ident(s)));

    let ident = choice((alphanumeric_ident, symbolic_ident));

    let directive = just('@')
        .ignore_then(filter(|c: &char| c.is_alphanumeric() || *c == '_').repeated().at_least(1).collect::<String>())
        .map(|s| RawToken::Tok(Token::Directive(s)));

    let lambda_char = just('λ').to(RawToken::Tok(Token::Lambda));

    let comment = just('#')
        .then(take_until(text::newline().or(end())))
        .map(|_| RawToken::Ignore);

    let newline = text::newline().to(RawToken::Tok(Token::Newline));
    let spaces = just(' ')
        .repeated()
        .at_least(1)
        .map(|v| RawToken::Spaces(v.len()));
    let tabs = just('\t')
        .repeated()
        .at_least(1)
        .map(|v| RawToken::Spaces(v.len() * 4)); // Assume 1 tab = 4 spaces for indent math

    let token = choice((
        num,
        string,
        op,
        directive,
        lambda_char,
        ident,
        comment,
        newline,
        spaces,
        tabs,
    ));

    token.map_with_span(|tok, span| (tok, span)).repeated()
}

pub enum LexMode {
    File,
    #[allow(dead_code)]
    Repl,
}

pub fn lex(input: &str, mode: LexMode) -> Result<Vec<(Token, Span)>, Vec<Simple<char>>> {
    let raw_tokens = raw_lexer().parse(input)?;
    
    // Filter out ignored tokens (comments)
    let raw_tokens: Vec<_> = raw_tokens.into_iter().filter(|(raw, _)| !matches!(raw, RawToken::Ignore)).collect();

    let mut tokens = Vec::new();
    let mut indent_stack = vec![0];
    let mut at_line_start = true;
    
    // State for tracking unclosed delimiters
    let mut open_delimiters = 0;
    
    // State for tracking line continuation
    let mut line_continued = false;

    for (raw, span) in raw_tokens {
        match raw {
            RawToken::Tok(Token::Ident(ref s)) if s == "\\" => {
                line_continued = true;
            }
            RawToken::Tok(Token::Newline) => {
                if line_continued {
                    line_continued = false;
                } else if open_delimiters > 0 {
                    // Ignore newlines inside delimiters
                } else {
                    if !at_line_start {
                        tokens.push((Token::Newline, span));
                    }
                    at_line_start = true;
                }
            }
            RawToken::Spaces(n) => {
                if at_line_start && !line_continued && open_delimiters == 0 {
                    let current_indent = *indent_stack.last().unwrap();
                    if n > current_indent {
                        indent_stack.push(n);
                        tokens.push((Token::Indent, span));
                    } else if n < current_indent {
                        while indent_stack.last().unwrap() > &n {
                            indent_stack.pop();
                            tokens.push((Token::Dedent, span.clone()));
                        }
                    }
                    at_line_start = false;
                }
            }
            RawToken::Tok(tok) => {
                if let Token::Ident(s) = &tok {
                    if s == "\\" {
                        continue; 
                    }
                }

                match tok {
                    Token::LParen | Token::LBracket | Token::LBrace => open_delimiters += 1,
                    Token::RParen | Token::RBracket | Token::RBrace => {
                        if open_delimiters > 0 {
                            open_delimiters -= 1;
                        }
                    }
                    _ => {}
                }

                if at_line_start && !line_continued && open_delimiters == 0 {
                    while *indent_stack.last().unwrap() > 0 {
                        indent_stack.pop();
                        tokens.push((Token::Dedent, span.clone()));
                    }
                    at_line_start = false;
                } else if at_line_start && (line_continued || open_delimiters > 0) {
                    at_line_start = false;
                }
                
                line_continued = false;
                tokens.push((tok, span));
            }
            RawToken::Ignore => unreachable!(),
        }
    }

    if matches!(mode, LexMode::Repl) && !at_line_start {
        let end_span = input.len()..input.len();
        tokens.push((Token::Newline, end_span));
    }

    while indent_stack.len() > 1 {
        indent_stack.pop();
        let end_span = input.len()..input.len();
        tokens.push((Token::Dedent, end_span));
    }

    let mut cleaned_tokens = Vec::new();
    let mut last_was_newline = true; 
    for (t, s) in tokens {
        if matches!(t, Token::Newline) {
            if !last_was_newline {
                cleaned_tokens.push((t, s));
                last_was_newline = true;
            }
        } else {
            cleaned_tokens.push((t, s));
            last_was_newline = false;
        }
    }

    Ok(cleaned_tokens)
}