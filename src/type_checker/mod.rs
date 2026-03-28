pub mod tast;
pub mod environment;
pub mod arity_resolver;
pub mod checker;
#[cfg(test)]
pub mod tests;

pub use checker::Checker;
