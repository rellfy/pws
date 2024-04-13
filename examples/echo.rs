use pws::{connect_persistent_websocket_async, Message, Url};
use std::str::FromStr;

#[tokio::main]
async fn main() {
    let (tx, mut rx) =
        connect_persistent_websocket_async(Url::from_str("wss://echo.websocket.org").unwrap())
            .await
            .unwrap();
    tokio::join!(
        async move {
            println!("sending: hello");
            tx.send(Message::Text("hello".to_owned())).unwrap();
            println!("closing connection");
            tx.send(Message::Close(None)).unwrap();
            println!("sending: hello again");
            tx.send(Message::Text("hello again".to_owned())).unwrap();
        },
        async move {
            while let Ok(msg) = rx.recv().await {
                let Message::Text(msg) = msg else {
                    continue;
                };
                println!("received: {msg}");
            }
        }
    );
}
