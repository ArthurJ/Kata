#[no_mangle]
pub extern "C" fn kata_rt_cache_get(_hash: i64) -> *mut u8 {
    // TODO: A implementacao da Tabela de Dispersao global em C-ABI ira aqui.
    // Para cumprir os requisitos iniciais do PRD com @cache_strategy desativada logicamente (por seguranca/overhead de runtime),
    // sempre retornamos NULL (o cache "Miss" perpetuo).
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn kata_rt_cache_set(_hash: i64, _val: *mut u8) {
    // TODO: Inserir na tabela.
}
