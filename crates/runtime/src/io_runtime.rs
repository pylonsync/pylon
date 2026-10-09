/// Worker threads for the connection runtime: the machine's cores, between
/// 2 and 8. Connections mostly wait; the shard tick threads do the work.
fn worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(2, 8)
}

/// The runtime every async connection runs on, whichever port it came in
/// on. Built on first use.
pub(crate) fn runtime() -> Option<&'static tokio::runtime::Runtime> {
    static RT: std::sync::OnceLock<Option<tokio::runtime::Runtime>> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(worker_threads())
            .thread_name("pylon-io")
            .enable_all()
            .build()
            .map_err(|e| tracing::warn!("[pylon-io] could not start the connection runtime: {e}"))
            .ok()
    })
    .as_ref()
}
