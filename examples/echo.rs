use pws::{connect_persistent_websocket_async, Message, Url, WsMessageReceiver, WsMessageSender};
use std::str::FromStr;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let url = Url::from_str("wss://echo.websocket.org").unwrap();
    let (tx, rx) = connect_persistent_websocket_async(url).await.unwrap();
    tokio::join!(send_messages(tx), receive_messages(rx),);
}

async fn send_messages(tx: WsMessageSender) {
    tx.send(Message::Text("hello".to_owned())).await.unwrap();
    sleep().await;
    // Simulate a connection close.
    tx.send(Message::Close(None)).await.unwrap();
    sleep().await;
    tx.send(Message::Text("hello again".to_owned()))
        .await
        .unwrap();
}

async fn receive_messages(mut rx: WsMessageReceiver) {
    while let Ok(msg) = rx.recv().await {
        match msg {
            Message::Text(msg) => {
                println!("received: {msg}");
                if msg == "hello again" {
                    std::process::exit(0);
                }
            }
            Message::ConnectionOpened => {
                println!("connection opened");
            }
            Message::ConnectionClosed => {
                println!("connection closed");
            }
            _ => {}
        }
    }
}

async fn sleep() {
    tokio::time::sleep(Duration::from_secs(3)).await;
}
