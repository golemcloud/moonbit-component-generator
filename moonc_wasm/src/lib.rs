mod wasmoo_extern;
pub mod cross_platform;

pub fn run_wasmoo(argv: Vec<String>) -> anyhow::Result<()> {
    let isolate = &mut v8::Isolate::new(Default::default());
    isolate.set_capture_stack_trace_for_uncaught_exceptions(true, 10);
    let scope = &mut v8::HandleScope::new(isolate);
    let context = v8::Context::new(scope, Default::default());
    // setup file descriptor table
    context.set_slot(wasmoo_extern::FdTable::new());
    // setup directory table
    // context.set_slot(wasmoo_extern::DirTable::new());
    let scope = &mut v8::ContextScope::new(scope, context);

    let global_proxy = scope.get_current_context().global(scope);
    wasmoo_extern::init_wasmoo(global_proxy, scope);

    let process_argv = v8::Array::new(scope, argv.len() as i32);
    for (i, s) in argv.iter().enumerate() {
        let s = v8::String::new(scope, s).unwrap();
        process_argv.set_index(scope, i as u32, s.into());
    }
    let ident = v8::String::new(scope, "process_argv").unwrap();
    global_proxy.set(scope, ident.into(), process_argv.into());

    let mut script = String::new();
    let js_glue = include_str!("moonc/js_glue_for_wasmoo.js");
    script.push_str(js_glue);
    let code = v8::String::new(scope, &script).unwrap();
    let name = v8::String::new(scope, "moonc").unwrap();
    let origin = v8::ScriptOrigin::new(
        scope,
        name.into(),
        0,
        0,
        false,
        0,
        None,
        false,
        false,
        false,
        None,
    );
    let code = v8::Script::compile(scope, code, Some(&origin)).unwrap();
    code.run(scope);
    Ok(())
}

pub fn initialize_v8() -> anyhow::Result<()> {
    v8::V8::set_flags_from_string(&format!("--stack-size={}", 102400));
    v8::V8::set_flags_from_string("--experimental-wasm-exnref");
    v8::V8::set_flags_from_string("--experimental-wasm-imported-strings");
    let platform = v8::new_default_platform(0, false).make_shared();
    v8::V8::initialize_platform(platform);
    v8::V8::initialize();
    Ok(())
}
