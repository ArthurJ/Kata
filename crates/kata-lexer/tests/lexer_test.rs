#[path = "lexer_test/comments.rs"]
mod comments;
#[path = "lexer_test/errors.rs"]
mod errors;

pub(crate) use helpers::tokens_only;
#[path = "lexer_test/helpers.rs"]
mod helpers;
#[path = "lexer_test/indent.rs"]
mod indent;
#[path = "lexer_test/integration.rs"]
mod integration;
#[path = "lexer_test/keywords.rs"]
mod keywords;
#[path = "lexer_test/numbers.rs"]
mod numbers;
#[path = "lexer_test/punct.rs"]
mod punct;
#[path = "lexer_test/spans.rs"]
mod spans;
#[path = "lexer_test/strings.rs"]
mod strings;
