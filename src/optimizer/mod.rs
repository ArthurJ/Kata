pub mod error;
pub mod passes;

use crate::type_checker::checker::TTopLevel;
use crate::parser::ast::Spanned;
use crate::type_checker::environment::TypeEnv;
use error::OptimizerError;

pub struct Optimizer<'a> {
    pub errors: Vec<OptimizerError>,
    pub env: &'a TypeEnv,
}

impl<'a> Optimizer<'a> {
    pub fn new(env: &'a TypeEnv) -> Self {
        Self {
            errors: Vec::new(),
            env,
        }
    }

    pub fn optimize(&mut self, tast: Vec<Spanned<TTopLevel>>, release_mode: bool) -> Vec<Spanned<TTopLevel>> {
        log::info!("Iniciando pipeline de otimização (MIR)...");
        
        let mut optimized_tast = tast;

        if release_mode {
            let mut shaker = passes::tree_shaker::TreeShaker::new(self.env);
            optimized_tast = shaker.run(optimized_tast);
        }

        let mut folder = passes::const_folder::ConstantFolder::new();
        optimized_tast = folder.run(optimized_tast);

        let mut comptime = passes::comptime::ComptimeEval::new(self.env);
        optimized_tast = comptime.run(optimized_tast, &mut self.errors);

        let mut monomorph = passes::monomorph::Monomorphizer::new(self.env);
        optimized_tast = monomorph.run(optimized_tast, &mut self.errors);

        let mut escape = passes::escape::EscapeAnalysis::new();
        optimized_tast = escape.run(optimized_tast, &mut self.errors);

        let mut tco = passes::tco::TcoPass::new();
        optimized_tast = tco.run(optimized_tast, &mut self.errors);

        let mut stream_fusion = passes::stream_fusion::StreamFusionPass::new();
        optimized_tast = stream_fusion.run(optimized_tast, &mut self.errors);

        if release_mode {
            let mut late_shaker = passes::tree_shaker::TreeShaker::new(self.env);
            optimized_tast = late_shaker.run(optimized_tast);
        }

        log::info!("Pipeline de otimização concluído.");
        optimized_tast
    }
}
