use notify_rust::{Notification, Timeout};

pub fn config_reload(success: bool) {
    let body = if success {
        "Configuration reloaded."
    } else {
        "Reload failed; the previous configuration is still active."
    };
    if let Err(error) = Notification::new()
        .summary("Fievel")
        .body(body)
        .timeout(Timeout::Milliseconds(4000))
        .show()
    {
        eprintln!("fievel: could not send desktop notification: {error}");
    }
}
