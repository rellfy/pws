# Pws
Persistent Websocket connection for tokio-tungstenite.

## Example
See more examples in the [examples](examples) folder.

```rust
use pws::{connect_persistent_websocket_async, Message, Url};
use std::str::FromStr;

#[tokio::main]
async fn main() {
    let url = Url::from_str("wss://echo.websocket.org").unwrap();
    let (tx, mut rx) = connect_persistent_websocket_async(url).await.unwrap();
    let mut close_count = 0;
    while let Ok(msg) = rx.recv().await {
        match msg {
            Message::Text(msg) => {
                println!("received: {msg}");
                tx.send(Message::Close(None)).await.unwrap();
            }
            Message::ConnectionOpened => {
                println!("connection opened ({close_count})");
                let msg = format!("hello! connection #{close_count}");
                tx.send(Message::Text(msg)).await.unwrap();
            }
            Message::ConnectionClosed => {
                println!("connection closed");
                close_count += 1;
                if close_count >= 2 {
                    break;
                }
            }
            _ => {}
        }
    }
    println!("persistent websocket is done");
}
```
