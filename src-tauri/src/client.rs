//! The desktop's RPC client (ADR-0009). It never hosts a server itself —
//! `supervisor` finds or spawns one, then hands this a socket to connect to.

use std::path::Path;
use std::sync::Arc;

pub use hyprlayer_server::client::{ConnectError, RpcClient};

pub async fn connect(app_data_dir: &Path) -> Result<Arc<RpcClient>, ConnectError> {
    let socket_path = hyprlayer_server::socket::socket_path(app_data_dir);
    RpcClient::connect_socket(&socket_path).await
}
