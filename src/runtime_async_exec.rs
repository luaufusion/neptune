use v8::ContextOptions;

/// Helper method to try compiling code to bytecode
fn compile_to_bytecode_bg(code: String) -> Option<Vec<u8>> {
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, ContextOptions::default());
    let mut scope = v8::ContextScope::new(scope, context);

    let code_v8 = v8::String::new(&mut scope, &code)?;
    let mut source = v8::script_compiler::Source::new(code_v8, None);

    let module = v8::script_compiler::compile_module2(
        &mut scope,
        &mut source,
        v8::script_compiler::CompileOptions::EagerCompile,
        v8::script_compiler::NoCacheReason::NoReason,
    )?;

    let cached_data = module.get_unbound_module_script(&mut scope);

    let cc = cached_data.create_code_cache()?;
    Some(cc.to_vec())
}
