use std::cell::Cell;
use std::rc::Rc;

use pipewire as pw;

fn main() -> Result<(), pw::Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None)?;
    let context = pw::context::Context::new(&mainloop)?;
    let core = context.connect(None)?;
    let registry = core.get_registry()?;
    let count = Rc::new(Cell::new(0));
    let c = count.clone();
    let _listener = registry
        .add_listener_local()
        .global(move |g| {
            if let Some(props) = g.props {
                if props.get("media.class").is_some_and(|m| m.starts_with("Audio/")) {
                    c.set(c.get() + 1);
                    println!("{} {}", props.get("media.class").unwrap_or(""), props.get("node.name").unwrap_or(""));
                }
            }
        })
        .register();
    let ml = mainloop.clone();
    let timer = mainloop.loop_().add_timer(move |_| ml.quit());
    timer.update_timer(Some(std::time::Duration::from_millis(500)), None).into_result()?;
    mainloop.run();
    println!("pipewire ok, {} audio nodes", count.get());
    Ok(())
}
