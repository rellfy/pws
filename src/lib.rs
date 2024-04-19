use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use log::{error, info};
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::{RecvError as BroadcastRecvError, SendError};
use tokio::sync::oneshot::error::RecvError as OneshotRecvError;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinError;
use tokio_tungstenite::tungstenite::protocol::frame::Frame;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
pub use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
pub use url::Url;

const INITIAL_BACKOFF_MILLIS: u64 = 100;
const MAX_BACKOFF_MILLIS: u64 = 5 * 60 * 1000;
const CHANNEL_CAPACITY: usize = 32;

type WsWrite = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>;
type WsRead = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;
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

pub type WsMessageSender = mpsc::Sender<Message>;
pub type WsMessageReceiver = broadcast::Receiver<Message>;

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close(Option<CloseFrame<'static>>),
    Frame(Frame),
    /// A message to notify that the connection was opened.
    /// Note that this cannot be sent, it is received only.
    /// This variant is only part of the Pws crate and not of
    /// tungstenite.
    ConnectionOpened,
    /// A message to notify that the connection was closed.
    /// Note that this cannot be sent, it is received only.
    /// This variant is only part of the Pws crate and not of
    /// tungstenite.
    ConnectionClosed,
}

pub async fn connect_persistent_websocket_async(
    url: Url,
) -> Result<(WsMessageSender, WsMessageReceiver), Error> {
    let (msg_tx_out, msg_rx_out) = mpsc::channel::<Message>(CHANNEL_CAPACITY);
    let (msg_tx_in, msg_rx_in) = broadcast::channel::<Message>(CHANNEL_CAPACITY);
    let (first_conn_tx, first_conn_rx) = oneshot::channel();
    tokio::spawn(setup_persistent_websocket(
        url.into(),
        msg_tx_in,
        msg_rx_out,
        first_conn_tx,
    ));
    if let Some(first_conn_error) = first_conn_rx.await? {
        return Err(Error::Tungstenite(first_conn_error));
    }
    Ok((msg_tx_out, msg_rx_in))
}

async fn setup_persistent_websocket(
    url: Url,
    mut msg_tx_in: broadcast::Sender<Message>,
    mut msg_rx_out: mpsc::Receiver<Message>,
    first_conn_tx: oneshot::Sender<Option<WsError>>,
) {
    let mut connection_count = 0;
    let mut first_conn_tx = Some(first_conn_tx);
    loop {
        info!("connecting to {url}");
        let result = listen_for_persistent_ws_messages(
            url.clone(),
            &mut msg_tx_in,
            &mut msg_rx_out,
            &mut first_conn_tx,
        )
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
    msg_tx_in: &mut broadcast::Sender<Message>,
    msg_rx_out: &mut mpsc::Receiver<Message>,
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
    msg_tx_in.send(Message::ConnectionOpened)?;
    let result = process_connection(&mut ws_tx, &mut ws_rx, msg_tx_in, msg_rx_out).await;
    msg_tx_in.send(Message::ConnectionClosed)?;
    result?;
    Ok(())
}

async fn process_connection(
    ws_tx: &mut WsWrite,
    ws_rx: &mut WsRead,
    msg_tx_in: &mut broadcast::Sender<Message>,
    msg_rx_out: &mut mpsc::Receiver<Message>,
) -> Result<(), Error> {
    loop {
        tokio::select! {
            Some(incoming_msg) = ws_rx.next() => {
                let should_close = handle_incoming_message(incoming_msg, ws_tx, msg_tx_in).await?;
                if should_close {
                    info!("closing connection");
                    break;
                }
            },
            Some(outgoing_msg) = msg_rx_out.recv() => {
                let Some(outgoing_msg) = outgoing_msg.to_tungstenite() else {
                    continue;
                };
                ws_tx.send(outgoing_msg).await?;
            }
        }
    }
    Ok(())
}

async fn handle_incoming_message(
    message: Result<TungsteniteMessage, WsError>,
    ws_tx: &mut WsWrite,
    msg_tx_in: &mut broadcast::Sender<Message>,
) -> Result<bool, Error> {
    let message = match message {
        Ok(m) => m,
        Err(e) => {
            error!("connection error: {e}");
            return Ok(true);
        }
    };
    match message {
        TungsteniteMessage::Ping(_) => {
            #[cfg(feature = "pong")]
            if let Err(e) = ws_tx.send(TungsteniteMessage::Pong(vec![])).await {
                error!("error sending pong: {e}");
            }
            return Ok(false);
        }
        TungsteniteMessage::Close(frame) => {
            info!("received socket close signal: {:#?}", frame);
            return Ok(true);
        }
        _ => {}
    }
    msg_tx_in.send(message.into())?;
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

impl Message {
    fn to_tungstenite(self) -> Option<TungsteniteMessage> {
        match self {
            Message::Text(v) => Some(TungsteniteMessage::Text(v)),
            Message::Binary(v) => Some(TungsteniteMessage::Binary(v)),
            Message::Ping(v) => Some(TungsteniteMessage::Ping(v)),
            Message::Pong(v) => Some(TungsteniteMessage::Pong(v)),
            Message::Close(v) => Some(TungsteniteMessage::Close(v)),
            Message::Frame(v) => Some(TungsteniteMessage::Frame(v)),
            Message::ConnectionOpened => None,
            Message::ConnectionClosed => None,
        }
    }
}

impl From<TungsteniteMessage> for Message {
    fn from(message: TungsteniteMessage) -> Self {
        match message {
            TungsteniteMessage::Text(v) => Message::Text(v),
            TungsteniteMessage::Binary(v) => Message::Binary(v),
            TungsteniteMessage::Ping(v) => Message::Ping(v),
            TungsteniteMessage::Pong(v) => Message::Pong(v),
            TungsteniteMessage::Close(v) => Message::Close(v),
            TungsteniteMessage::Frame(v) => Message::Frame(v),
        }
    }
}
