pub mod context;
pub mod translator;
pub mod expr;
pub mod linker;

#[cfg(test)]
mod tests;

use crate::type_checker::checker::TTopLevel;
use crate::parser::ast::Spanned;

pub fn compile_and_link(tast: Vec<Spanned<TTopLevel>>, output_bin: &str) -> Result<(), String> {
    log::info!("Iniciando Backend/Codegen Cranelift (AOT)");

    let object_file = format!("{}.o", output_bin);

    // 1. Setup Context
    let mut ctx = context::CodegenContext::new(&object_file)?;

    // 2. Translate TAST to Cranelift IR
    let mut translator = translator::FunctionTranslator::new(&mut ctx);
    translator.translate(tast)?;

    // 3. Finalize Module and emit object file
    ctx.finish()?;

    // 4. Link object file with runtime
    linker::link_executable(&object_file, output_bin)?;

    Ok(())
}
