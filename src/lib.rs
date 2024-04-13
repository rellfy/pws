use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use log::{error, info};
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::{RecvError as BroadcastRecvError, SendError};
use tokio::sync::oneshot::error::RecvError as OneshotRecvError;
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use url::Url;

const INITIAL_BACKOFF_MILLIS: u64 = 100;
const MAX_BACKOFF_MILLIS: u64 = 5 * 60 * 1000;
const CHANNEL_CAPACITY: usize = 32;

type WsWrite = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type WsError = tokio_tungstenite::tungstenite::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("tungstenite: {0}")]
    Tungstenite(WsError),
    #[error("broadcast::recv: {0}")]
    BroadcastRecv(BroadcastRecvError),
    #[error("oneshot::recv: {0}")]
    OneshotRecv(OneshotRecvError),
    #[error("join: {0}")]
    Join(JoinError),
    #[error("broadcast::send: {0}")]
    Send(SendError<Message>),
}

pub type WsMessageSender = broadcast::Sender<Message>;
pub type WsMessageReceiver = broadcast::Receiver<Message>;

pub async fn connect_persistent_websocket_async<T>(
    url: T,
) -> Result<(WsMessageSender, WsMessageReceiver), Error>
where
    T: Into<Url>,
{
    let (msg_tx, msg_rx) = broadcast::channel::<Message>(CHANNEL_CAPACITY);
    let (first_conn_tx, first_conn_rx) = oneshot::channel();
    tokio::spawn(setup_persistent_websocket(
        url.into(),
        msg_tx.clone(),
        first_conn_tx,
    ));
    if let Some(first_conn_error) = first_conn_rx.await? {
        return Err(Error::Tungstenite(first_conn_error));
    }
    Ok((msg_tx, msg_rx))
}

async fn setup_persistent_websocket(
    url: Url,
    msg_tx: broadcast::Sender<Message>,
    first_conn_tx: oneshot::Sender<Option<WsError>>,
) {
    let mut connection_count = 0;
    let mut first_conn_tx = Some(first_conn_tx);
    loop {
        info!("connecting to {url}");
        let msg_tx = msg_tx.clone();
        let msg_rx = msg_tx.subscribe();
        let result =
            listen_for_persistent_ws_messages(url.clone(), msg_tx, msg_rx, &mut first_conn_tx)
                .await;
        if let Err(e) = result {
            error!("error during ws connection: {e}");
        }
        info!("disconnected from {url}");
        connection_count += 1;
        tokio::time::sleep(get_backoff(connection_count)).await;
    }
}

async fn listen_for_persistent_ws_messages(
    url: Url,
    mut msg_tx: broadcast::Sender<Message>,
    mut msg_rx: broadcast::Receiver<Message>,
    first_conn_tx: &mut Option<oneshot::Sender<Option<WsError>>>,
) -> Result<(), Error> {
    let connection_result = connect_async(url).await;
    let (socket, _) = match connection_result {
        Ok(c) => {
            if let Some(first_conn_tx) = first_conn_tx.take() {
                first_conn_tx.send(None).expect("first_conn_rx dropped")
            }
            c
        }
        Err(e) => {
            if let Some(first_conn_tx) = first_conn_tx.take() {
                first_conn_tx.send(Some(e)).expect("first_conn_rx dropped");
            }
            return Ok(());
        }
    };
    let (mut ws_tx, mut ws_rx) = socket.split();
    loop {
        tokio::select! {
            Some(incoming_msg) = ws_rx.next() => {
                let should_close = handle_message(incoming_msg, &mut ws_tx, &mut msg_tx).await?;
                if should_close {
                    break;
                }
            },
            outgoing_msg = msg_rx.recv() => {
                ws_tx.send(outgoing_msg?).await?;
            }
        }
    }
    Ok(())
}

async fn handle_message(
    message: Result<Message, WsError>,
    ws_tx: &mut WsWrite,
    msg_tx: &mut broadcast::Sender<Message>,
) -> Result<bool, Error> {
    let message = match message {
        Ok(m) => m,
        Err(e) => {
            error!("connection error: {e}");
            return Ok(true);
        }
    };
    match message {
        Message::Ping(_) => {
            #[cfg(feature = "pong")]
            if let Err(e) = ws_tx.send(Message::Pong(vec![])).await {
                error!("error sending pong: {e}");
            }
            return Ok(true);
        }
        Message::Close(frame) => {
            info!("received socket close signal: {:#?}", frame);
            return Ok(true);
        }
        _ => {}
    }
    msg_tx.send(message)?;
    Ok(false)
}

fn get_backoff(attempt: u64) -> Duration {
    let backoff = INITIAL_BACKOFF_MILLIS * attempt.pow(2);
    Duration::from_millis(backoff.min(MAX_BACKOFF_MILLIS))
}

impl From<WsError> for Error {
    fn from(e: WsError) -> Self {
        Self::Tungstenite(e)
    }
}

impl From<BroadcastRecvError> for Error {
    fn from(e: BroadcastRecvError) -> Self {
        Self::BroadcastRecv(e)
    }
}

impl From<OneshotRecvError> for Error {
    fn from(e: OneshotRecvError) -> Self {
        Self::OneshotRecv(e)
    }
}

impl From<JoinError> for Error {
    fn from(e: JoinError) -> Self {
        Self::Join(e)
    }
}

impl From<SendError<Message>> for Error {
    fn from(e: SendError<Message>) -> Self {
        Self::Send(e)
    }
}
