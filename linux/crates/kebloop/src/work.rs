//! Blocking work (compositor IPC, D-Bus, HTTP, subprocesses) runs on a worker thread; the result
//! comes back through a channel and is delivered on the GTK main loop.

use gtk::glib;

/// Run `work` on a named worker thread and call `done` with its result on the main loop.
/// Returns `false` (and drops `done`) when the thread could not be started.
pub fn spawn<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) -> bool {
    let (sender, receiver) = async_channel::bounded::<T>(1);
    let spawned = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let _ = sender.send_blocking(work());
        });
    if let Err(error) = spawned {
        eprintln!("kebloop: could not start {name}: {error}");
        return false;
    }
    glib::spawn_future_local(async move {
        if let Ok(result) = receiver.recv().await {
            done(result);
        }
    });
    true
}
