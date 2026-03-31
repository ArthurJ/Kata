use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::ir::{AbiParam, types};
use cranelift_object::{ObjectBuilder, ObjectModule};
use cranelift_module::{default_libcall_names, FuncId, Linkage, Module};
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

    /// Declare common FFI functions from the prelude that may be called
    pub fn declare_prelude_ffi(&mut self) -> Result<(), String> {
        // FFI functions: (external_name, kata_action_name, params, return)
        // Note: For FFI actions, we use the external name for declaration and store both mappings
        let ffi_functions: Vec<(&str, Vec<cranelift_codegen::ir::Type>, Option<cranelift_codegen::ir::Type>)> = vec![
            // I/O functions
            ("kata_rt_print_str", vec![types::I64], None), // pointer to string
            // Channel functions
            ("kata_rt_chan_create_rendezvous", vec![], Some(types::I64)), // returns channel pair
            ("kata_rt_channel_send_async", vec![types::I64, types::I64, types::I64], Some(types::I8)), // handle, value, waker -> bool
            ("kata_rt_channel_recv_async", vec![types::I64, types::I64], Some(types::I64)), // handle, waker -> value
            ("kata_rt_channel_recv_try", vec![types::I64], Some(types::I64)), // handle -> value
            // System functions
            ("kata_rt_sleep_sync", vec![types::I64], None), // ms
            // Type functions
            ("kata_rt_default_repr", vec![types::I64], Some(types::I64)), // pointer -> pointer to string
            ("kata_rt_eq_generic", vec![types::I64, types::I64], Some(types::I8)), // a, b -> bool
            // String functions
            ("kata_rt_str_int", vec![types::I64], Some(types::I64)),
            ("kata_rt_str_float", vec![types::F64], Some(types::I64)),
            ("kata_rt_str_text", vec![types::I64], Some(types::I64)),
            ("kata_rt_str_bool", vec![types::I8], Some(types::I64)),
            // Math functions
            ("kata_rt_pow_int", vec![types::I64, types::I64], Some(types::I64)),
            ("kata_rt_pow_float", vec![types::F64, types::F64], Some(types::F64)),
            // Fork
            ("kata_rt_fork", vec![types::I64], None),
        ];

        for (name, params, ret) in ffi_functions {
            let mut sig = self.module.make_signature();
            for param_type in &params {
                sig.params.push(AbiParam::new(*param_type));
            }
            if let Some(ret_type) = ret {
                sig.returns.push(AbiParam::new(ret_type));
            }

            let func_id = self.module.declare_function(name, Linkage::Import, &sig)
                .map_err(|e| format!("Falha ao declarar FFI `{}`: {}", name, e))?;
            self.functions.insert(name.to_string(), func_id);
        }

        // Map common action names to their FFI implementations
        // This allows calls like echo! to find kata_rt_print_str
        let ffi_mappings: Vec<(&str, &str)> = vec![
            ("echo", "kata_rt_print_str"),
            ("sleep", "kata_rt_sleep_sync"),
            ("channel", "kata_rt_chan_create_rendezvous"),
        ];

        for (action_name, ffi_name) in ffi_mappings {
            // Map action name with no suffix to FFI
            if let Some(&func_id) = self.functions.get(ffi_name) {
                self.functions.insert(action_name.to_string(), func_id);
                // Also map variadic versions
                self.functions.insert(format!("{}_Var_Text", action_name), func_id);
                self.functions.insert(format!("{}_Text", action_name), func_id);
            }
        }

        Ok(())
    }

    pub fn finish(self) -> Result<(), String> {
        let product = self.module.finish();
        let bytes = product.emit().map_err(|e| format!("Falha ao emitir arquivo objeto: {}", e))?;
        std::fs::write(&self.object_filename, bytes).map_err(|e| format!("Falha ao escrever arquivo objeto: {}", e))?;
        Ok(())
    }
}
