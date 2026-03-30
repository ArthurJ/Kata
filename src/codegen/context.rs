use cranelift_codegen::settings::{self, Configurable};
use cranelift_object::{ObjectBuilder, ObjectModule};
use cranelift_module::Module;
use cranelift_module::default_libcall_names;
use cranelift_module::FuncId;
use std::collections::HashMap;

pub struct CodegenContext {
    pub module: ObjectModule,
    pub functions: HashMap<String, FuncId>,
    pub object_filename: String,
}

impl CodegenContext {
    pub fn new(object_filename: &str) -> Result<Self, String> {
        let mut flag_builder = settings::builder();
        flag_builder.set("is_pic", "true").unwrap();
        flag_builder.set("opt_level", "speed_and_size").unwrap();

        let isa_builder = cranelift_native::builder().map_err(|e| format!("Falha ao obter ISA nativo: {}", e))?;
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .map_err(|e| format!("Falha ao criar ISA: {}", e))?;

        let builder = ObjectBuilder::new(
            isa,
            "kata_module",
            default_libcall_names(),
        ).map_err(|e| format!("Falha ao criar ObjectBuilder: {}", e))?;

        let module = ObjectModule::new(builder);

        Ok(Self {
            module,
            functions: HashMap::new(),
            object_filename: object_filename.to_string(),
        })
    }

    pub fn finish(self) -> Result<(), String> {
        let product = self.module.finish();
        let bytes = product.emit().map_err(|e| format!("Falha ao emitir arquivo objeto: {}", e))?;
        std::fs::write(&self.object_filename, bytes).map_err(|e| format!("Falha ao escrever arquivo objeto: {}", e))?;
        Ok(())
    }
}
