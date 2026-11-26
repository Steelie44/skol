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
