//! Integration tests for kata-parser.
//!
//! These tests exercise the parser through the public `parse` API,
//! lexing source strings and verifying the resulting AST structure.

mod action_type_syntax;
mod actions;
mod basics;
mod collections;
mod csp;
mod dict_set;
mod directive_values;
mod guards;
mod helpers;
mod lambdas;
mod match_tests;
mod nesting_depth;
mod pipe;
mod qualified_variant;
mod refined_decls;
mod signatures;
