//! Abstração de backend de codegen — trait `ModuleBackend`.
//!
//! `ModuleBackend` extends `cranelift_module::Module` com `finalize()`.
//! O lowering usa `&mut dyn ModuleBackend` para ser agnóstico ao backend
//! (JIT hoje, AOT posteriormente). O pipeline específico — execução JIT,
//! emissão de object file — vive fora do lowering, em `jit.rs` e `aot.rs`,
//! onde o backend concreto é conhecido.
//!
//! Design: `ModuleBackend: Module` evita reimplementar a API de declarar/
//! definir/referenciar funções e data. Cada backend delega os 9 métodos
//! obrigatórios de `Module` para o module interno. O custo é ~30 linhas de
//! boilerplate de delegação; o ganho é que o lowering inteiro fica agnóstico
//! ao backend sem alteração de lógica.
//!
//! # `finalize()` e ownership
//!
//! `JITModule::finalize_definitions(&mut self)` mantém o module vivo (é
//! `&mut self`), permitindo `get_finalized_function(&self)` depois.
//! `ObjectModule::finish(self) -> ObjectProduct` **consome** o module.
//! Para acomodar ambos sob o mesmo trait (`finalize(&mut self)`),
//! `AotBackend` guarda `Option<ObjectModule>` e `Option<ObjectProduct>`:
//! `finalize()` faz `take()` do inner e armazena o produto; `emit()`
//! recupera o produto. Após `finalize()`, chamadas a métodos de `Module`
//! em `AotBackend` panicam — mas o pipeline AOT nunca as faz (só chama
//! `emit()`), espelhando o JIT que só chama `get_finalized_function`.

use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::{Context, ir, isa};
use cranelift_module::{
    DataDescription, DataId, FuncId, Linkage, Module, ModuleDeclarations, ModuleResult,
};

use super::module::CodegenError;

/// Backend de codegen — abstrai JIT vs AOT.
///
/// Extends `Module` com `finalize()`. O lowering usa apenas os métodos de
/// `Module` (declarar, definir, referenciar funções/data). `finalize()` é
/// chamado pelo pipeline após o lowering, antes de obter o resultado
/// (fn pointer JIT, bytes AOT).
pub(crate) trait ModuleBackend: Module {
    /// Finaliza todas as definições — resolve relocations, compila machine
    /// code (JIT) ou emite object file (AOT).
    fn finalize(&mut self) -> Result<(), CodegenError>;
}

// ─────────────────────────────────────────────────────────────────────
// JIT
// ─────────────────────────────────────────────────────────────────────

/// Backend JIT — wrap `cranelift_jit::JITModule`.
///
/// O lowering recebe `&mut JitBackend` coagido para `&mut dyn ModuleBackend`.
/// O pipeline JIT (`jit_eval`, `jit_compile_tests`) cria o `JitBackend`,
/// chama o lowering, `finalize()`, e então `get_finalized_function()` para
/// obter o fn pointer — método específico de JIT que não pertence ao trait.
pub(crate) struct JitBackend {
    inner: cranelift_jit::JITModule,
}

impl JitBackend {
    pub(crate) fn new(inner: cranelift_jit::JITModule) -> Self {
        Self { inner }
    }

    /// Consome o wrapper e retorna o `JITModule` interno.
    /// Usado quando o pipeline precisa de `std::mem::forget(module)` para
    /// manter as páginas de código mapeadas após a execução.
    pub(crate) fn into_inner(self) -> cranelift_jit::JITModule {
        self.inner
    }

    /// Obtém o ponteiro de código de uma função finalizada.
    /// Específico de JIT — AOT emite bytes, não ponteiros.
    pub(crate) fn get_finalized_function(&self, fid: FuncId) -> *const u8 {
        self.inner.get_finalized_function(fid)
    }
}

// ── Delegação `Module` → `JITModule` interno ──────────────────────────
// Boilerplate necessário: `Module` não tem blanket impl para wrappers.
// 9 métodos obrigatórios + os que têm default impl são cobertos pelo trait.

impl Module for JitBackend {
    fn isa(&self) -> &dyn isa::TargetIsa {
        self.inner.isa()
    }

    fn declarations(&self) -> &ModuleDeclarations {
        self.inner.declarations()
    }

    fn declare_function(
        &mut self,
        name: &str,
        linkage: Linkage,
        signature: &ir::Signature,
    ) -> ModuleResult<FuncId> {
        self.inner.declare_function(name, linkage, signature)
    }

    fn declare_anonymous_function(&mut self, signature: &ir::Signature) -> ModuleResult<FuncId> {
        self.inner.declare_anonymous_function(signature)
    }

    fn declare_data(
        &mut self,
        name: &str,
        linkage: Linkage,
        writable: bool,
        tls: bool,
    ) -> ModuleResult<DataId> {
        self.inner.declare_data(name, linkage, writable, tls)
    }

    fn declare_anonymous_data(&mut self, writable: bool, tls: bool) -> ModuleResult<DataId> {
        self.inner.declare_anonymous_data(writable, tls)
    }

    fn define_function_with_control_plane(
        &mut self,
        func: FuncId,
        ctx: &mut Context,
        ctrl_plane: &mut ControlPlane,
    ) -> ModuleResult<()> {
        self.inner
            .define_function_with_control_plane(func, ctx, ctrl_plane)
    }

    fn define_function_bytes(
        &mut self,
        func_id: FuncId,
        alignment: u64,
        bytes: &[u8],
        relocs: &[cranelift_module::ModuleReloc],
    ) -> ModuleResult<()> {
        self.inner
            .define_function_bytes(func_id, alignment, bytes, relocs)
    }

    fn define_data(&mut self, data_id: DataId, data: &DataDescription) -> ModuleResult<()> {
        self.inner.define_data(data_id, data)
    }
}

impl ModuleBackend for JitBackend {
    fn finalize(&mut self) -> Result<(), CodegenError> {
        self.inner
            .finalize_definitions()
            .map_err(|e| CodegenError::Cranelift {
                reason: format!("finalize_definitions: {e}"),
            })
    }
}

// ─────────────────────────────────────────────────────────────────────
// AOT
// ─────────────────────────────────────────────────────────────────────

/// Backend AOT — wrap `cranelift_object::ObjectModule`.
///
/// `ObjectModule::finish(self)` consome o module, então guardamos
/// `Option<ObjectModule>` e `Option<ObjectProduct>`. O lowering usa apenas
/// os métodos de `Module` (via `&mut dyn ModuleBackend`); após `finalize()`,
/// o pipeline chama `emit()` para obter os bytes do object file.
pub(crate) struct AotBackend {
    inner: Option<cranelift_object::ObjectModule>,
    product: Option<cranelift_object::ObjectProduct>,
}

impl AotBackend {
    pub(crate) fn new(inner: cranelift_object::ObjectModule) -> Self {
        Self {
            inner: Some(inner),
            product: None,
        }
    }

    /// Emite os bytes do object file após `finalize()`.
    ///
    /// Específico de AOT — JIT retorna ponteiros, não bytes. Consome o
    /// `ObjectProduct` armazenado por `finalize()`.
    pub(crate) fn emit(&mut self) -> Result<Vec<u8>, CodegenError> {
        let product = self.product.take().ok_or_else(|| CodegenError::Cranelift {
            reason: "emit() chamado antes de finalize()".into(),
        })?;
        product.emit().map_err(|e| CodegenError::Cranelift {
            reason: format!("object emit: {e}"),
        })
    }
}

// ── Delegação `Module` → `ObjectModule` interno ───────────────────────
// Após `finalize()`, `inner` é `None` — chamadas panicam. O pipeline AOT
// nunca chama métodos de `Module` após `finalize()` (só `emit()`), espelhando
// o JIT que só chama `get_finalized_function`.

impl Module for AotBackend {
    fn isa(&self) -> &dyn isa::TargetIsa {
        self.inner
            .as_ref()
            .expect("Module::isa após finalize() — bug do pipeline AOT")
            .isa()
    }

    fn declarations(&self) -> &ModuleDeclarations {
        self.inner
            .as_ref()
            .expect("Module::declarations após finalize() — bug do pipeline AOT")
            .declarations()
    }

    fn declare_function(
        &mut self,
        name: &str,
        linkage: Linkage,
        signature: &ir::Signature,
    ) -> ModuleResult<FuncId> {
        let inner = self
            .inner
            .as_mut()
            .expect("Module::declare_function após finalize() — bug do pipeline AOT");
        inner.declare_function(name, linkage, signature)
    }

    fn declare_anonymous_function(&mut self, signature: &ir::Signature) -> ModuleResult<FuncId> {
        let inner = self
            .inner
            .as_mut()
            .expect("Module::declare_anonymous_function após finalize() — bug do pipeline AOT");
        inner.declare_anonymous_function(signature)
    }

    fn declare_data(
        &mut self,
        name: &str,
        linkage: Linkage,
        writable: bool,
        tls: bool,
    ) -> ModuleResult<DataId> {
        let inner = self
            .inner
            .as_mut()
            .expect("Module::declare_data após finalize() — bug do pipeline AOT");
        inner.declare_data(name, linkage, writable, tls)
    }

    fn declare_anonymous_data(&mut self, writable: bool, tls: bool) -> ModuleResult<DataId> {
        let inner = self
            .inner
            .as_mut()
            .expect("Module::declare_anonymous_data após finalize() — bug do pipeline AOT");
        inner.declare_anonymous_data(writable, tls)
    }

    fn define_function_with_control_plane(
        &mut self,
        func: FuncId,
        ctx: &mut Context,
        ctrl_plane: &mut ControlPlane,
    ) -> ModuleResult<()> {
        let inner = self
            .inner
            .as_mut()
            .expect("Module::define_function após finalize() — bug do pipeline AOT");
        inner.define_function_with_control_plane(func, ctx, ctrl_plane)
    }

    fn define_function_bytes(
        &mut self,
        func_id: FuncId,
        alignment: u64,
        bytes: &[u8],
        relocs: &[cranelift_module::ModuleReloc],
    ) -> ModuleResult<()> {
        let inner = self
            .inner
            .as_mut()
            .expect("Module::define_function_bytes após finalize() — bug do pipeline AOT");
        inner.define_function_bytes(func_id, alignment, bytes, relocs)
    }

    fn define_data(&mut self, data_id: DataId, data: &DataDescription) -> ModuleResult<()> {
        let inner = self
            .inner
            .as_mut()
            .expect("Module::define_data após finalize() — bug do pipeline AOT");
        inner.define_data(data_id, data)
    }
}

impl ModuleBackend for AotBackend {
    fn finalize(&mut self) -> Result<(), CodegenError> {
        let inner = self.inner.take().ok_or_else(|| CodegenError::Cranelift {
            reason: "finalize() chamado duas vezes".into(),
        })?;
        self.product = Some(inner.finish());
        Ok(())
    }
}
