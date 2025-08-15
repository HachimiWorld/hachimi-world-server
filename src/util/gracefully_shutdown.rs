use tokio::signal;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub fn gen_cancel_token() -> (CancellationToken, JoinHandle<()>) {
    let cancel_token = CancellationToken::new();
    let cancel_handle = tokio::spawn({
        let cancel_token = cancel_token.clone();
        async move {
            tokio::select! {
                result = shutdown_signal() => {
                    match result {
                        Ok(_) => {}
                        Err(err) => error!("Install signal handler failed: {:?}", err)
                    };
                    info!("Received shutdown signal");
                    cancel_token.cancel(); 
                }
                _ = cancel_token.cancelled() => {}
            }
        }
    });
    (cancel_token, cancel_handle)
}

async fn shutdown_signal() -> anyhow::Result<()> {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    /*let ci_notify = async {
        // Wait for CI signal
        // ...
    }*/

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
        // _ = ci_notify => {}
    }
    Ok(())
}