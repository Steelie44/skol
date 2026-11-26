//! A minimal game host: exposes a `game` module to Skol, loads the script and
//! runs its `update` function every frame.
//!
//!     cargo run -p skol --example game_loop

use std::cell::RefCell;
use std::rc::Rc;

use skol::{Engine, Limits, Vec2};

fn main() -> Result<(), skol::Error> {
    // Host-side game state the script can affect only through the API below.
    let entities = Rc::new(RefCell::new(Vec::<(String, Vec2)>::new()));
    let frame = Rc::new(RefCell::new(0u32));

    let mut engine = Engine::new();
    // Bound each frame's script work: a buggy or hostile script can't hang the game.
    engine.set_limits(Limits { max_steps: Some(1_000_000), ..Limits::default() });

    let ents = entities.clone();
    engine.register("game", "spawn", move |kind: String, at: Vec2| -> i64 {
        ents.borrow_mut().push((kind, at));
        10 // points for spawning, returned to the script
    })?;
    let f = frame.clone();
    engine.register("game", "key_down", move |key: String| -> bool {
        // Pretend the player holds "right" for the first 40 frames.
        key == "right" && *f.borrow() < 40
    })?;
    let f = frame.clone();
    engine.register("game", "draw_text", move |text: String, at: Vec2| {
        if *f.borrow() % 15 == 0 { println!("frame {:3}: draw {text:?} at ({}, {})", f.borrow(), at.x, at.y); }
    })?;

    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/scripts/game.skl");
    engine.load_file(script)?;

    for _ in 0..60 {
        engine.call::<(), _>("update", (1.0 / 60.0,))?;
        *frame.borrow_mut() += 1;
    }

    println!("final score: {}", engine.get::<i64>("enemies")? * 10);
    println!("entities spawned by the script: {:?}", entities.borrow());
    Ok(())
}
