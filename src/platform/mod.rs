pub mod windows;

pub(crate) fn reap(worker: Option<std::thread::JoinHandle<()>>) {
    let Some(worker) = worker else {
        return;
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
    while !worker.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    if worker.is_finished() {
        let _ = worker.join();
    } else {
        eprintln!(
            "Timed out waiting for {}",
            worker.thread().name().unwrap_or("worker")
        );
    }
}
