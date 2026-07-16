//! Integration tests for kata-parser.
//!
//! These tests exercise the parser through the public `parse` API,
//! lexing source strings and verifying the resulting AST structure.

mod actions;
mod basics;
mod fio6;
mod fio8_collections;
mod guards;
mod helpers;
mod lambdas;
mod match_tests;
mod pipe;
mod signatures;
