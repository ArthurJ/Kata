//! Dispatch por método de interface (Caminho 0 do `infer_apply`).
//!
//! Quando um argumento é tipado como `Ty::Interface("SHOW")` e `func_name`
//! é método dessa interface (ex: `show`), retorna o tipo de retorno do método
//! com `Self` substituído pelo tipo da interface.
//!
//! Separado de `apply.rs` por ser self-contained: só recebe
//! `(func_name, arg_types, iface_reg)` e não compartilha state com
//! `InferCtx` ou `TypeEnv`.

use kata_core::ty::Ty;

/// Tenta dispatch por método de interface.
///
/// Quando um argumento é tipado como `Ty::Interface("SHOW")` e `func_name`
/// é método dessa interface (ex: `show`), retorna o tipo de retorno do método
/// com `Self` substituído pelo tipo da interface. Caso contrário, retorna `None`.
///
/// Ex: `show msg` onde `msg :: Interface("SHOW")`:
/// - InterfaceRegistry tem `SHOW { signatures: [show :: Self => Text] }`
/// - Substitui `Self` por `Interface("SHOW")` → retorna `Text`
///
/// O callee produzido é `Ident("show")` sem overload específico — o
/// monomorphizador resolve a impl concreta ao instanciar a Action
/// polimórfica que contém esta chamada.
pub(crate) fn try_iface_method_dispatch(
    func_name: &str,
    arg_types: &[Ty],
    iface_reg: &kata_core::InterfaceRegistry,
) -> Option<Ty> {
    // Para cada arg, verifica se é Ty::Interface(name) ou Ty::Var(name).
    //
    // Ty::Interface(name): procura a interface `name` no registry e verifica
    // se `func_name` é um de seus métodos. Substitui `Self` por
    // `Interface(name)` para obter o tipo de retorno.
    //
    // Ty::Var(name): asserção implícita "todo Ty::Var implementa SHOW"
    // (e qualquer interface — todo tipo concreto implementará). Retorna
    // o tipo de retorno do método se a interface existir e tiver o método.
    // O despacho concreto é resolvido pelo monomorphizador ao instanciar.
    for arg in arg_types {
        match arg {
            Ty::Interface(iface_name) => {
                if let Some(iface_info) = iface_reg.get_interface(iface_name) {
                    for sig in &iface_info.signatures {
                        if sig.name == func_name && sig.params.len() == arg_types.len() {
                            let mut params_match = true;
                            for (sp, sa) in sig.params.iter().zip(arg_types) {
                                if matches!(sp, Ty::Var(name) if name == "Self") {
                                    if !matches!(sa, Ty::Interface(n) if n == iface_name) {
                                        params_match = false;
                                        break;
                                    }
                                } else if sp != sa {
                                    params_match = false;
                                    break;
                                }
                            }
                            if params_match {
                                return Some(sig.ret.clone());
                            }
                        }
                    }
                }
            }
            Ty::Var(_) => {
                // Ty::Var em função genérica sintetizada (ex: `show v` dentro
                // de `__kata_show__Result`). Assumimos que o tipo concreto
                // que substituirá o Var implementa a interface. Procuramos
                // qualquer interface que tenha `func_name` como método e
                // cujo número de params case, e retornamos seu `ret`.
                // O despacho concreto é resolvido no monomorphizador.
                for iface_info in iface_reg.all_interfaces() {
                    for sig in &iface_info.signatures {
                        if sig.name == func_name && sig.params.len() == arg_types.len() {
                            // Verifica params: Self deve ser Ty::Var, outros
                            // devem bater com arg_types.
                            let mut params_match = true;
                            for (sp, sa) in sig.params.iter().zip(arg_types) {
                                if matches!(sp, Ty::Var(name) if name == "Self") {
                                    // Self → Ty::Var aceita
                                    if !matches!(sa, Ty::Var(_)) {
                                        params_match = false;
                                        break;
                                    }
                                } else if sp != sa {
                                    params_match = false;
                                    break;
                                }
                            }
                            if params_match {
                                return Some(sig.ret.clone());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}
