use pws::{connect_persistent_websocket_async, Message, Url};
use std::str::FromStr;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let (tx, mut rx) =
        connect_persistent_websocket_async(Url::from_str("wss://echo.websocket.org").unwrap())
            .await
            .unwrap();
    tokio::join!(
        async move {
            println!("sending: hello");
            tx.send(Message::Text("hello".to_owned())).await.unwrap();
            tokio::time::sleep(Duration::from_secs(3)).await;
            println!("closing connection");
            tx.send(Message::Close(None)).await.unwrap();
            tokio::time::sleep(Duration::from_secs(3)).await;
            println!("sending: hello again");
            tx.send(Message::Text("hello again".to_owned()))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(3)).await;
        },
        async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    Message::Text(msg) => {
                        println!("received: {msg}");
                        if &msg == "hello again" {
                            println!("exiting");
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
    );
}
