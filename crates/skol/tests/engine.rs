use std::cell::RefCell;
use std::rc::Rc;

use skol::{Engine, Error, Limits, Vec2};

fn captured(engine: &mut Engine) -> Rc<RefCell<String>> {
    let out = Rc::new(RefCell::new(String::new()));
    let sink = out.clone();
    engine.on_print(move |s| sink.borrow_mut().push_str(s));
    out
}

#[test]
fn host_functions_are_typed_and_callable() {
    let mut engine = Engine::new();
    let spawned = Rc::new(RefCell::new(Vec::new()));
    let log = spawned.clone();
    engine.register("game", "spawn", move |kind: String, at: Vec2| -> i64 {
        log.borrow_mut().push((kind, at));
        log.borrow().len() as i64
    }).unwrap();
    engine.register("game", "dt", || -> f64 { 0.5 }).unwrap();
    let out = captured(&mut engine);
    engine.load_source("main.skl", r#"
        from game use spawn, dt
        var count: i = 0
        fn update(frames: i) -> i {
            for k in range(frames) {
                count = spawn("slime", {float(k), 2.0})
            }
            print("dt", dt())
            return count
        }
    "#).unwrap();
    let n: i64 = engine.call("update", (3i64,)).unwrap();
    assert_eq!(n, 3);
    assert_eq!(spawned.borrow()[2], ("slime".to_string(), Vec2::new(2.0, 2.0)));
    assert_eq!(engine.get::<i64>("count").unwrap(), 3);
    assert_eq!(out.borrow().as_str(), "dt 0.5\n");
}

#[test]
fn scripts_calling_the_host_api_wrongly_fail_to_compile() {
    let mut engine = Engine::new();
    engine.register("game", "spawn", |_kind: String, _at: Vec2| -> i64 { 0 }).unwrap();
    let err = engine.load_source("main.skl", "from game use spawn\nspawn(1, 2)").unwrap_err();
    assert!(matches!(err, Error::Compile(_)), "{err}");
    assert!(err.to_string().contains("argument 1 of game.spawn(): expected s, found i"), "{err}");

    let mut engine = Engine::new();
    let err = engine.load_source("main.skl", "from game use spawn").unwrap_err();
    assert!(err.to_string().contains("can't find module 'game'"), "{err}");
}

#[test]
fn calls_from_rust_are_type_checked() {
    let mut engine = Engine::new();
    engine.load_source("main.skl", "fn add(a: i, b: i) -> i { return a + b }").unwrap();
    assert_eq!(engine.call::<i64, _>("add", (2i64, 40i64)).unwrap(), 42);
    let e = engine.call::<i64, _>("add", (2.0f64, 40i64)).unwrap_err();
    assert!(e.to_string().contains("add takes (i, i), but was called with (f, i)"), "{e}");
    let e = engine.call::<String, _>("add", (1i64, 1i64)).unwrap_err();
    assert!(e.to_string().contains("add returns i, not s"), "{e}");
    assert!(engine.call::<(), _>("missing", ()).is_err());
}

#[test]
fn globals_can_be_read_and_written() {
    let mut engine = Engine::new();
    engine.load_source("main.skl", "var speed: f = 1.5\nconst NAME: s = \"ada\"\nfn doubled() -> f { return speed * 2.0 }").unwrap();
    assert_eq!(engine.get::<f64>("speed").unwrap(), 1.5);
    engine.set("speed", 4.0f64).unwrap();
    assert_eq!(engine.call::<f64, _>("doubled", ()).unwrap(), 8.0);
    assert_eq!(engine.get::<String>("NAME").unwrap(), "ada");
    assert!(engine.set("NAME", "x".to_string()).unwrap_err().to_string().contains("constant"));
    assert!(engine.set("speed", 1i64).is_err());
}

#[test]
fn file_access_is_off_unless_allowed() {
    let mut engine = Engine::new();
    engine.load_source("main.skl", "from io use read_file\nfn peek() -> s { return read_file(\"Cargo.toml\") }").unwrap();
    let e = engine.call::<String, _>("peek", ()).unwrap_err();
    assert!(e.to_string().contains("file access is disabled"), "{e}");
    engine.allow_file_access(true);
    assert!(engine.call::<String, _>("peek", ()).unwrap().contains("[package]"));
}

#[test]
fn limits_stop_runaway_scripts_and_the_engine_survives() {
    let mut engine = Engine::new();
    engine.set_limits(Limits { max_steps: Some(10_000), ..Limits::default() });
    engine.load_source("main.skl", "fn spin() { while true { pass } }\nfn ok() -> i { return 7 }\nfn deep(n: i) -> i { return deep(n + 1) }").unwrap();
    let e = engine.call::<(), _>("spin", ()).unwrap_err();
    assert!(e.to_string().contains("step limit exceeded"), "{e}");
    assert_eq!(engine.call::<i64, _>("ok", ()).unwrap(), 7);
    engine.set_limits(Limits { max_depth: 100, ..Limits::default() });
    let e = engine.call::<i64, _>("deep", (0i64,)).unwrap_err();
    assert!(e.to_string().contains("stack overflow"), "{e}");
    assert_eq!(engine.call::<i64, _>("ok", ()).unwrap(), 7);
}

#[test]
fn host_errors_and_panics_become_script_errors() {
    let mut engine = Engine::new();
    engine.register("game", "load_level", |n: i64| -> Result<(), String> {
        if n > 3 { Err(format!("level {n} doesn't exist")) } else { Ok(()) }
    }).unwrap();
    engine.register("game", "broken", || -> i64 { panic!("host bug") }).unwrap();
    engine.load_source("main.skl", "from game use load_level, broken\nfn go(n: i) { load_level(n) }\nfn crash() -> i { return broken() }").unwrap();
    engine.call::<(), _>("go", (2i64,)).unwrap();
    let e = engine.call::<(), _>("go", (9i64,)).unwrap_err();
    assert!(matches!(e, Error::Runtime(_)));
    assert!(e.to_string().contains("level 9 doesn't exist") && e.to_string().contains("in go"), "{e}");
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let e = engine.call::<i64, _>("crash", ()).unwrap_err();
    std::panic::set_hook(prev);
    assert!(e.to_string().contains("host function game.broken panicked"), "{e}");
    engine.call::<(), _>("go", (1i64,)).unwrap();
}

#[test]
fn input_comes_from_the_host() {
    let mut engine = Engine::new();
    let mut lines = vec!["second".to_string(), "first".to_string()];
    engine.on_input(move || lines.pop());
    engine.load_source("main.skl", "fn read2() -> s { return input() + \"/\" + input() + \"/\" + input() }").unwrap();
    assert_eq!(engine.call::<String, _>("read2", ()).unwrap(), "first/second/");
}

#[test]
fn api_misuse_is_reported_not_panicked() {
    let mut engine = Engine::new();
    assert!(engine.call::<(), _>("update", ()).is_err());
    assert!(engine.get::<i64>("x").is_err());
    engine.load_source("main.skl", "var x: i = 1").unwrap();
    assert!(engine.load_source("again.skl", "var y: i = 2").is_err());
    assert!(engine.register("game", "late", || -> i64 { 0 }).is_err());
}

#[test]
fn os_features_are_sandboxed_by_default() {
    for (src, what) in [
        ("from os use env\nvar x: s = env(\"PATH\")\n", "env is disabled"),
        ("from os use exit\nexit(1)\n", "exit is disabled"),
        ("from time use sleep\nsleep(0.0)\n", "sleep is disabled"),
        ("from io use list_dir\nvar t: T8[s] = list_dir(\".\")\n", "file access is disabled"),
    ] {
        let mut engine = Engine::new();
        match engine.load_source("main.skl", src) {
            Err(Error::Runtime(e)) => assert!(e.message.contains(what), "{src}: {}", e.message),
            other => panic!("{src}: expected a runtime error, got {other:?}"),
        }
    }
}

#[test]
fn allow_os_enables_exit_and_args() {
    let mut engine = Engine::new();
    engine.allow_os(true).set_args(vec!["level2".into(), "hard".into()]);
    let out = captured(&mut engine);
    engine.load_source("main.skl", r#"
        from os use arg_count, arg, exit
        fn run() -> i {
            print(arg_count(), arg(0), arg(1))
            exit(7)
            return 0
        }
    "#).unwrap();
    match engine.call::<i64, _>("run", ()) {
        Err(Error::Exit(7)) => {}
        other => panic!("expected Exit(7), got {other:?}"),
    }
    assert_eq!(out.borrow().as_str(), "2 level2 hard\n");
}

#[test]
fn library_paths_are_searched_after_the_script_folder() {
    let lib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib");
    let mut engine = Engine::new();
    let out = captured(&mut engine);
    engine.add_library_path(&lib);
    engine.load_source("main.skl", r#"
        from fsm use StateMachine
        var m: StateMachine = StateMachine()
        m.add_state("idle")
        m.on_enter("idle", lambda() -> void { print("entered") })
        m.start("idle")
    "#).unwrap();
    assert_eq!(out.borrow().as_str(), "entered\n");

    let mut plain = Engine::new();
    match plain.load_source("main.skl", "from fsm use StateMachine\n") {
        Err(Error::Compile(d)) => assert!(d.message.contains("can't find module 'fsm'"), "{}", d.message),
        other => panic!("expected a compile error, got {other:?}"),
    }
}

#[test]
fn dynamic_calls_check_types_at_run_time() {
    use skol::Dyn;
    let mut engine = Engine::new();
    engine.load_source("main.skl", r#"
        var speed: f = 1.5
        const MAX: i = 3
        fn scale(v: vec2, k: f) -> vec2 { return v * k }
        fn greet(name: s, times: i) -> s { return name.repeat(times) }
    "#).unwrap();
    assert_eq!(engine.call_dynamic("scale", &[Dyn::Vec2(Vec2::new(1.0, 2.0)), Dyn::Int(2)]).unwrap(), Dyn::Vec2(Vec2::new(2.0, 4.0)));
    assert_eq!(engine.call_dynamic("greet", &[Dyn::Str("ab".into()), Dyn::Int(2)]).unwrap(), Dyn::Str("abab".into()));
    assert!(matches!(engine.call_dynamic("greet", &[Dyn::Int(1), Dyn::Int(2)]), Err(Error::Api(m)) if m.contains("argument 1 of greet must be s")));
    assert!(matches!(engine.call_dynamic("nope", &[]), Err(Error::Api(_))));
    engine.set_dynamic("speed", &Dyn::Int(4)).unwrap();
    assert_eq!(engine.get_dynamic("speed").unwrap(), Dyn::Float(4.0));
    assert!(engine.set_dynamic("MAX", &Dyn::Int(1)).is_err());
    assert_eq!(engine.function_types("scale").unwrap().0.len(), 2);
}
