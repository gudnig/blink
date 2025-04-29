use std::sync::Arc;

use tokio::net::TcpStream;

use crate::session::Session;

pub async fn handle_lsp(session: Arc<Session>, mut socket: TcpStream) {}
