use std::sync::Arc;
use sha2::{Sha256, Digest};
use rand::{Rng, SeedableRng, rngs::StdRng};
use futures_util::{StreamExt, SinkExt, stream::{SplitSink, SplitStream}};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{self, Duration};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message, WebSocketStream};

struct TimeState {
    seed: u64,
    previous_hash: String,
    coin_flip_result: bool,
    current_hash: String,
}

fn generate_time_hash(seed: u64, previous_hash: &str, coin_flip: bool) -> String {
    // create the hasher
    let mut hasher = Sha256::new();

    // hash is seed, previous hash, coin flip
    hasher.update(seed.to_string());
    hasher.update(previous_hash);
    hasher.update(coin_flip.to_string());

    // Truncate the hash for simplicity
    format!("{:x}", hasher.finalize())[..8].to_string()
}

async fn time_update_task(mut seed: u64, time_state: Arc<Mutex<TimeState>>, ticks_per_day: u64) {
    let mut rng: StdRng = SeedableRng::seed_from_u64(seed);
    let mut tick_counter = 0u64;

    loop {
        // Use a specific time interval for the tick (i.e. 1024 milliseconds)
        let interval = time::sleep(Duration::from_millis(1024));
        interval.await;

        // get the state
        let mut state = time_state.lock().await;

        // If the number of ticks reaches the specified limit, roll over to a new day
        if tick_counter >= ticks_per_day {
            // reset the tick_coutner
            tick_counter = 0;

            // increment the seed
            seed += 1;

            // reset the hash, for a new day
            state.previous_hash = "00000000".to_string();
        }

        // calculate the state
        let new_previous_hash = state.current_hash.clone();
        state.coin_flip_result = rng.gen_bool(0.5);
        state.current_hash = generate_time_hash(seed, &state.previous_hash, state.coin_flip_result);
        state.previous_hash = new_previous_hash;
        state.seed = seed;

        // update the ticks
        tick_counter += 1;
    }
}

async fn register_bot(write: &mut SplitSink<WebSocketStream<impl AsyncRead + AsyncWrite + Unpin>, Message>, bot_name: &str) {
    let registration_message = Message::Text(format!("register as {}", bot_name));
    write.send(registration_message).await.expect("Failed to send registration message");
}

fn parse_message_data(message_data: &str) -> Option<(&str, &str)> {
    let sender_id_match = message_data.find(", message: ");
    sender_id_match.map(|index| {
        let (sender_id, message_content) = message_data.split_at(index);
        let sender_id = &sender_id["from: ".len()..];
        let message_content = &message_content[", message: ".len()..];
        (sender_id, message_content)
    })
}

async fn handle_incoming_messages(mut read: SplitStream<WebSocketStream<impl AsyncRead + AsyncWrite + Unpin>>, mut write: SplitSink<WebSocketStream<impl AsyncRead + AsyncWrite + Unpin>, Message>, time_state: Arc<Mutex<TimeState>>) {
    // loop through messages
    while let Some(message) = read.next().await {
        match message {
            Ok(Message::Text(text)) => {
                // parse the message
                if let Some((sender_id, _)) = parse_message_data(&text) {
                    // get the state
                    let state = time_state.lock().await;

                    // do the coin flip
                    let coin_flip_str = if state.coin_flip_result { "H" } else { "T" };

                    // format the time (seed, previous_hash, coin_flip, current_hash)
                    let time_representation = format!("{}:{}:{}:{}", state.seed, state.previous_hash, coin_flip_str, state.current_hash);

                    // send the current time
                    write.send(Message::Text(format!("@{} Current time: {}", sender_id, time_representation))).await.expect("Failed to send message");
                }
            }
            Err(e) => eprintln!("Error receiving message: {}", e),
            _ => {} // Ignore other messages
        }
    }
}


#[tokio::main]
async fn main() {
    let url = "ws://localhost:3000";

    println!("Connecting to - {}", url);
    let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
    println!("Connected to Agent Network");

    let (mut write, mut read) = ws_stream.split();

    // register the timebot
    register_bot(&mut write, "RustTimeBot").await;

    // set the initial day
    let day_seed = 12345u64;

    // setup the initial state
    let initial_state = TimeState {
        seed: day_seed,
        previous_hash: "00000000".to_string(),
        coin_flip_result: false,
        current_hash: "00000000".to_string(),
    };

    // set the initial time
    let time_state = Arc::new(Mutex::new(initial_state));

    // kick off time updates
    let ticks_per_day = 1048576;
    tokio::spawn(time_update_task(day_seed, time_state.clone(), ticks_per_day));

    // Handle incoming messages in a separate task
    let read_handle = tokio::spawn(handle_incoming_messages(read,write,time_state));

    // Await both tasks (optional, depending on your use case)
    let _ = tokio::try_join!(read_handle);
}
