use std::io::IsTerminal;

pub(crate) fn init(json: bool) {
    let subscriber = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_max_level(tracing::Level::INFO);

    if json {
        let _ = subscriber
            .json()
            .flatten_event(true)
            .with_ansi(false)
            .try_init();
    } else {
        let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        let _ = subscriber.with_ansi(color).try_init();
    }
}
